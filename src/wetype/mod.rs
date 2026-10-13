//! WeType/wx-client protocol ported from ../wetype-x11/main.go.
//! Upstream protocol: xifan2333/vinput-registry, wetype/streaming/entry.py.
//! This is an unofficial cloud service; audio is sent to wetype.weixin.qq.com.
mod segment;
mod wire;

use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use openssl::{
    bn::{BigNum, BigNumContext},
    ec::{EcGroup, EcKey, EcPoint, PointConversionForm},
    hash::{hash, MessageDigest},
    nid::Nid,
    rand::rand_bytes,
    symm::{decrypt, encrypt, Cipher},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpStream,
    sync::mpsc,
    time::{sleep, timeout, Instant},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tracing::warn;
use uuid::Uuid;

use crate::config::Config;
use crate::websocket::AsrEvent;
use wire::{bytes, fields, get_bytes, get_number, number, MAX_MESSAGE};

const HOST: &str = "wetype.weixin.qq.com";
const SIGN_KEY: &str = "zN7rB3bL4pO8jW1o";
const BOOT_KEY: &[u8] = b"D4Y5U3Y2M0C0T7N4P1P7O2N6E1I2Y1U6";
const CMD_DH: u64 = 2147483646;
const CMD_UIN: u64 = 0x7ffffdfd;
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const FINISH_GRACE: Duration = Duration::from_secs(5);

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect()
}

fn random_lower(n: usize) -> Result<String> {
    let mut data = vec![0; n];
    rand_bytes(&mut data)?;
    Ok(data
        .into_iter()
        .map(|b| char::from(b'a' + b % 26))
        .collect())
}

#[derive(Serialize, Deserialize)]
struct Identity {
    device: String,
    uin: serde_json::Value,
}

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    key: Vec<u8>,
    task: u64,
    uin: String,
    device: String,
}

impl Client {
    async fn connect() -> Result<Self> {
        let mut request = format!("wss://{HOST}/").into_client_request()?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", "wxws_pb".parse()?);
        let (ws, _) = timeout(IO_TIMEOUT, connect_async(request))
            .await
            .context("WeType connection timed out")??;
        Ok(Self {
            ws,
            key: Vec::new(),
            task: 0,
            uin: "0".into(),
            device: String::new(),
        })
    }

    async fn roundtrip(
        &mut self,
        path: &str,
        body: &[u8],
        cmd: u64,
        compression: &str,
        token: &str,
    ) -> Result<(Vec<u8>, BTreeMap<String, String>)> {
        timeout(
            IO_TIMEOUT,
            self.roundtrip_inner(path, body, cmd, compression, token),
        )
        .await
        .with_context(|| format!("WeType request timed out at {path}"))?
    }

    async fn roundtrip_inner(
        &mut self,
        path: &str,
        body: &[u8],
        cmd: u64,
        compression: &str,
        token: &str,
    ) -> Result<(Vec<u8>, BTreeMap<String, String>)> {
        let special = cmd == CMD_DH || cmd == CMD_UIN;
        let task = if special {
            cmd
        } else {
            self.task += 1;
            self.task
        };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .to_string();
        let trace = random_lower(16)?;
        let digest = hex(&hash(MessageDigest::md5(), body)?);
        let mut values = vec!["5".into(), "2.2.3(657)".into(), "2".into()];
        if special {
            values.extend([ts.clone(), digest.clone()]);
            if cmd == CMD_UIN {
                values.push(token.into());
            }
            values.extend([trace.clone(), cmd.to_string()]);
        } else {
            values.extend([
                cmd.to_string(),
                "0".into(),
                ts.clone(),
                digest.clone(),
                self.uin.clone(),
                trace.clone(),
                task.to_string(),
            ]);
        }
        let sign = hex(&hash(
            MessageDigest::sha256(),
            (values.concat() + SIGN_KEY).as_bytes(),
        )?);
        let mut headers = BTreeMap::from([
            ("Kb-Uin", self.uin.clone()),
            ("Kb-OsType", "5".into()),
            ("Kb-Version", "2.2.3(657)".into()),
            ("Kb-SystemVersion", "27.0.0".into()),
            ("Kb-PackageType", "3".into()),
            ("Use_DebugNet", "0".into()),
            ("Kb-TimeStamp", ts),
            ("Kb-BodyMd5", digest),
            ("Kb-TraceId", trace),
            ("Kb-TaskId", task.to_string()),
            ("Kb-Scene", "2".into()),
            ("Kb-Sign", sign),
            ("Content-Length", body.len().to_string()),
            ("Kb-CompressionType", compression.into()),
            ("Content-Type", "application/octet-stream".into()),
            ("HOST", HOST.into()),
        ]);
        if !token.is_empty() {
            headers.insert("Kb-GenUinToken", token.into());
        }
        if special {
            headers.insert("Kb-DeviceCodeRestrictionV2", "1".into());
        }
        if cmd != CMD_DH {
            if !self.key.is_empty() {
                headers.insert(
                    "Kb-SharedKeySuffix",
                    String::from_utf8(self.key[self.key.len() - 4..].to_vec())?,
                );
            }
            headers.insert("Kb-CmdId", cmd.to_string());
            headers.insert("Kb-SubCmdId", "0".into());
        }
        let mut http = Vec::new();
        bytes(&mut http, 1, "POST");
        bytes(&mut http, 3, path);
        bytes(&mut http, 4, "");
        for (key, value) in headers {
            let mut entry = Vec::new();
            bytes(&mut entry, 1, key);
            bytes(&mut entry, 2, value);
            bytes(&mut http, 5, entry);
        }
        bytes(&mut http, 6, body);
        let mut msg = Vec::new();
        number(&mut msg, 1, 0);
        number(&mut msg, 2, 0);
        number(&mut msg, 3, task);
        bytes(&mut msg, 5, http);
        self.ws.send(Message::Binary(msg)).await?;
        let raw = loop {
            match self.ws.next().await.context("WeType connection ended")?? {
                Message::Binary(data) => break data,
                Message::Ping(data) => self.ws.send(Message::Pong(data)).await?,
                Message::Close(_) => bail!("WeType connection closed"),
                _ => {}
            }
        };
        let outer = fields(&raw)?;
        let resp = fields(get_bytes(&outer, 4).context("WeType response missing container")?)?;
        let status = get_number(&resp, 2).unwrap_or(0);
        if status != 200 {
            bail!("WeType HTTP status {status} at {path}");
        }
        let mut headers = BTreeMap::new();
        for (num, value) in &resp {
            if *num == 5 {
                continue;
            }
            if let wire::Value::Bytes(b) = value {
                if let Ok(kv) = fields(b) {
                    if let (Some(k), Some(v)) = (get_bytes(&kv, 1), get_bytes(&kv, 2)) {
                        headers.insert(
                            String::from_utf8(k.to_vec())?.to_ascii_lowercase(),
                            String::from_utf8(v.to_vec())?,
                        );
                    }
                }
            }
        }
        Ok((get_bytes(&resp, 5).unwrap_or_default().to_vec(), headers))
    }

    async fn exchange(&mut self, register: bool) -> Result<(String, String)> {
        // The reference's custom constants are the standard secp128r1 curve.
        let group = EcGroup::from_curve_name(Nid::SECP128R1)?;
        let local = EcKey::generate(&group)?;
        let mut ctx = BigNumContext::new()?;
        let public = hex(&local.public_key().to_bytes(
            &group,
            PointConversionForm::UNCOMPRESSED,
            &mut ctx,
        )?);
        let mut req = Vec::new();
        bytes(&mut req, 1, &public);
        bytes(&mut req, 2, &public);
        bytes(&mut req, 4, &self.device);
        bytes(&mut req, 7, "");
        bytes(&mut req, 8, "Mac16,12");
        bytes(&mut req, 9, "");
        if register {
            number(&mut req, 3, 1);
        } else {
            number(&mut req, 5, self.uin.parse()?);
        }
        let enc = encrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &req)?;
        let (body, _) = self
            .roundtrip("/oauth_pubkey_v2", &enc, CMD_DH, "1", "")
            .await?;
        let plain = decrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &body)?;
        let reply = fields(&plain)?;
        let server =
            std::str::from_utf8(get_bytes(&reply, 2).context("WeType server key missing")?)?
                .to_string();
        self.key = derive_key(&group, local.private_key(), &server)?;
        let token = std::str::from_utf8(get_bytes(&reply, 4).unwrap_or_default())?.to_string();
        Ok((server, token))
    }

    async fn handshake(&mut self, identity_path: &Path) -> Result<()> {
        self.roundtrip("/timestamp", &[], 0, "1", "").await?;
        let mut cached = false;
        if let Ok(data) = std::fs::read(identity_path) {
            if let Ok(identity) = serde_json::from_slice::<Identity>(&data) {
                let uin = identity
                    .uin
                    .as_u64()
                    .or_else(|| identity.uin.as_str().and_then(|s| s.parse().ok()))
                    .unwrap_or(0);
                if !identity.device.is_empty() && uin > 0 {
                    self.device = identity.device;
                    self.uin = uin.to_string();
                    cached = self.exchange(false).await.is_ok();
                }
            }
        }
        if !cached {
            self.uin = "0".into();
            self.key.clear();
            let body = format!("MAC{}Mac16,12{}", "0".repeat(9), random_lower(12)?);
            self.device = format!(
                "{body}{}",
                hex(&hash(
                    MessageDigest::md5(),
                    (body.clone() + SIGN_KEY).as_bytes()
                )?)
            );
            let (server, token) = self.exchange(true).await?;
            if token.is_empty() {
                bail!("WeType registration token missing");
            }
            let mut req = Vec::new();
            bytes(&mut req, 1, &self.device);
            bytes(&mut req, 2, &server);
            bytes(&mut req, 4, &token);
            let enc = encrypt(Cipher::aes_256_ecb(), &self.key, None, &req)?;
            let (body, _) = self
                .roundtrip("/gen_uin_v2", &enc, CMD_UIN, "1", &token)
                .await?;
            let plain = decrypt(Cipher::aes_256_ecb(), &self.key, None, &body)?;
            let reply = fields(&plain)?;
            let uin = get_number(&reply, 2)
                .filter(|n| *n > 0)
                .context("WeType registration failed")?;
            self.uin = uin.to_string();
            save_identity(
                identity_path,
                &Identity {
                    device: self.device.clone(),
                    uin: uin.into(),
                },
            )?;
        }
        let enc = encrypt(Cipher::aes_256_ecb(), &self.key, None, &[])?;
        self.roundtrip("/api_v2", &enc, 8074, "1", "").await?;
        Ok(())
    }

    async fn voice(
        &mut self,
        id: &str,
        opus: &[u8],
        seq: u64,
        total: u64,
        end: bool,
    ) -> Result<(String, String)> {
        let mut req = Vec::new();
        bytes(&mut req, 2, id);
        number(&mut req, 5, 5);
        number(&mut req, 7, seq);
        for n in [22, 23, 24] {
            number(&mut req, n, 1);
        }
        if !opus.is_empty() {
            bytes(&mut req, 4, opus);
        }
        if end {
            number(&mut req, 6, 1);
        }
        if total > 0 {
            number(&mut req, 11, total);
        }
        let mut inner = Vec::new();
        bytes(&mut inner, 1, req);
        let compressed = snap::raw::Encoder::new().compress_vec(&inner)?;
        let enc = encrypt(Cipher::aes_256_ecb(), &self.key, None, &compressed)?;
        let (body, headers) = self.roundtrip("/api_v2", &enc, 4548, "2", "").await?;
        let mut plain = decrypt(Cipher::aes_256_ecb(), &self.key, None, &body)?;
        if headers
            .get("kb-compressiontype")
            .or_else(|| headers.get("compressiontype"))
            .map(String::as_str)
            == Some("2")
        {
            if snap::raw::decompress_len(&plain)? > MAX_MESSAGE {
                bail!("WeType snappy response too large");
            }
            plain = snap::raw::Decoder::new().decompress_vec(&plain)?;
        }
        let mut reply = fields(&plain)?;
        if let Some(inner) = get_bytes(&reply, 1) {
            reply = fields(inner)?;
        }
        Ok((
            std::str::from_utf8(get_bytes(&reply, 4).unwrap_or_default())?.into(),
            std::str::from_utf8(get_bytes(&reply, 14).unwrap_or_default())?.into(),
        ))
    }
}

fn derive_key(group: &EcGroup, private: &openssl::bn::BigNumRef, public: &str) -> Result<Vec<u8>> {
    if public.len() != 66 || !public.starts_with("04") || !public.is_ascii() {
        bail!("Invalid WeType server key");
    }
    let data: Vec<u8> = (0..public.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&public[i..i + 2], 16))
        .collect::<std::result::Result<_, _>>()?;
    let mut ctx = BigNumContext::new()?;
    let peer = EcPoint::from_bytes(group, &data, &mut ctx)?;
    if peer.is_infinity(group) || !peer.is_on_curve(group, &mut ctx)? {
        bail!("Invalid WeType server point");
    }
    let mut shared = EcPoint::new(group)?;
    shared.mul(group, &peer, private, &ctx)?;
    if shared.is_infinity(group) {
        bail!("Invalid WeType shared point");
    }
    let (mut x, mut y) = (BigNum::new()?, BigNum::new()?);
    shared.affine_coordinates_gfp(group, &mut x, &mut y, &mut ctx)?;
    Ok(hex(&x.to_vec_padded(16)?).into_bytes())
}

fn save_identity(path: &Path, identity: &Identity) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // Write a new 0600 file then atomically replace, also fixing permissive old files.
    let tmp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(identity)?)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result.with_context(|| format!("Failed to save WeType identity at {}", path.display()))
}

struct AudioFramer {
    encoder: opus::Encoder,
    packets: Vec<Vec<u8>>,
    seq: u64,
    total: u64,
}

impl AudioFramer {
    fn new() -> Result<Self> {
        let mut encoder =
            opus::Encoder::new(16000, opus::Channels::Mono, opus::Application::Audio)?;
        encoder.set_bitrate(opus::Bitrate::Bits(64000))?;
        encoder.set_complexity(9)?;
        Ok(Self {
            encoder,
            packets: Vec::new(),
            seq: 0,
            total: 0,
        })
    }

    fn encode_frame(&mut self, frame: &[u8]) -> Result<()> {
        let samples: Vec<i16> = frame
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        self.packets.push(self.encoder.encode_vec(&samples, 4000)?);
        Ok(())
    }

    fn take(&mut self) -> Vec<u8> {
        self.seq += 1;
        let mut framed = Vec::new();
        if self.seq == 1 {
            framed.extend_from_slice(b"#!OPUS_RAW_V1\x02\x01\x00");
        }
        for packet in self.packets.drain(..) {
            framed.extend_from_slice(&(packet.len() as u16).to_le_bytes());
            framed.extend_from_slice(&packet);
        }
        self.total += framed.len() as u64;
        framed
    }
}

async fn partial(events: &mpsc::Sender<AsrEvent>, latest: &mut String, text: String) -> Result<()> {
    if !text.is_empty() && *latest != text {
        *latest = text.clone();
        events
            .send(AsrEvent::ResultGenerated {
                text,
                is_final: false,
            })
            .await?;
    }
    Ok(())
}

pub async fn start_recognition(
    config: &Config,
    audio: mpsc::Receiver<Vec<u8>>,
    events: mpsc::Sender<AsrEvent>,
) -> Result<()> {
    // Thirty seconds of pending PCM frames, bounded independently of network
    // finalization. The producer keeps detecting pauses while the worker polls.
    let (segment_tx, segment_rx) = mpsc::channel(1500);
    let silence_ms = config.wetype_segment_silence_ms;
    let threshold = config.wetype_voice_threshold;
    let producer = tokio::spawn(segment::produce(audio, segment_tx, silence_ms, threshold));
    let result = recognize_segments(config, segment_rx, &events).await;
    if result.is_err() {
        producer.abort();
    }
    let producer_result = producer.await;
    result?;
    producer_result??;
    events.send(AsrEvent::TaskFinished).await?;
    Ok(())
}

struct Utterance {
    id: String,
    framer: AudioFramer,
    latest: String,
}

impl Utterance {
    fn new() -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().simple().to_string(),
            framer: AudioFramer::new()?,
            latest: String::new(),
        })
    }
}

async fn recognize_segments(
    config: &Config,
    audio: mpsc::Receiver<segment::AudioEvent>,
    events: &mpsc::Sender<AsrEvent>,
) -> Result<()> {
    let mut client = Client::connect().await?;
    client.handshake(&config.wetype_identity).await?;
    events.send(AsrEvent::TaskStarted).await?;
    process_segments(&mut client, audio, events).await?;
    let _ = timeout(Duration::from_secs(1), client.ws.close(None)).await;
    Ok(())
}

async fn process_segments(
    client: &mut Client,
    mut audio: mpsc::Receiver<segment::AudioEvent>,
    events: &mpsc::Sender<AsrEvent>,
) -> Result<()> {
    let mut current: Option<Utterance> = None;
    while let Some(event) = audio.recv().await {
        match event {
            segment::AudioEvent::Frame(frame) => {
                if current.is_none() {
                    current = Some(Utterance::new()?);
                }
                let utterance = current.as_mut().unwrap();
                utterance.framer.encode_frame(&frame)?;
                if utterance.framer.packets.len() != 6 {
                    continue;
                }
                let framed = utterance.framer.take();
                let (raw, _) = client
                    .voice(
                        &utterance.id,
                        &framed,
                        utterance.framer.seq,
                        utterance.framer.total,
                        false,
                    )
                    .await?;
                partial(events, &mut utterance.latest, raw).await?;
            }
            segment::AudioEvent::End => {
                if let Some(utterance) = current.take() {
                    finish_utterance(client, utterance, events).await?;
                }
            }
        }
    }
    if let Some(utterance) = current.take() {
        finish_utterance(client, utterance, events).await?;
    }
    Ok(())
}

async fn finish_utterance(
    client: &mut Client,
    utterance: Utterance,
    events: &mpsc::Sender<AsrEvent>,
) -> Result<()> {
    let Utterance {
        id,
        mut framer,
        mut latest,
    } = utterance;
    if framer.seq > 0 || !framer.packets.is_empty() {
        let framed = framer.take();
        let finish = client
            .voice(&id, &framed, framer.seq, framer.total, true)
            .await;
        let mut polished = String::new();
        match finish {
            Ok((raw, final_text)) => {
                partial(events, &mut latest, raw).await?;
                polished = final_text;
                let deadline = Instant::now() + FINISH_GRACE;
                for _ in 0..8 {
                    if !polished.is_empty() {
                        break;
                    }
                    sleep(Duration::from_millis(250)).await;
                    if Instant::now() >= deadline {
                        break;
                    }
                    match timeout(
                        deadline - Instant::now(),
                        client.voice(&id, &[], 0, framer.total, true),
                    )
                    .await
                    {
                        Ok(Ok((raw, final_text))) => {
                            partial(events, &mut latest, raw).await?;
                            polished = final_text;
                        }
                        _ => break,
                    }
                }
            }
            Err(e) if !latest.is_empty() => {
                warn!("WeType finish failed; using latest partial: {e}")
            }
            Err(e) => return Err(e),
        }
        let text = if polished.is_empty() {
            latest
        } else {
            polished
        };
        if !text.is_empty() {
            events
                .send(AsrEvent::ResultGenerated {
                    text,
                    is_final: true,
                })
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn paused_segments_finalize_in_order_without_retyping() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut ids = Vec::new();
            for final_text in ["第一句。", "第二句。"] {
                let msg = ws.next().await.unwrap().unwrap().into_data();
                let outer = fields(&msg).unwrap();
                let request = fields(get_bytes(&outer, 5).unwrap()).unwrap();
                let compressed = decrypt(
                    Cipher::aes_256_ecb(),
                    BOOT_KEY,
                    None,
                    get_bytes(&request, 6).unwrap(),
                )
                .unwrap();
                let plain = snap::raw::Decoder::new()
                    .decompress_vec(&compressed)
                    .unwrap();
                let inner = fields(&plain).unwrap();
                let voice = fields(get_bytes(&inner, 1).unwrap()).unwrap();
                ids.push(get_bytes(&voice, 2).unwrap().to_vec());
                assert_eq!(get_number(&voice, 7), Some(1));
                assert_eq!(get_number(&voice, 6), Some(1));
                assert!(get_bytes(&voice, 4).unwrap().starts_with(b"#!OPUS_RAW_V1"));
                let mut text = Vec::new();
                bytes(&mut text, 4, "临时文本");
                bytes(&mut text, 14, final_text);
                let enc = encrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &text).unwrap();
                let mut response = Vec::new();
                number(&mut response, 2, 200);
                bytes(&mut response, 5, enc);
                let mut outer = Vec::new();
                bytes(&mut outer, 4, response);
                ws.send(Message::Binary(outer)).await.unwrap();
            }
            assert_ne!(ids[0], ids[1]);
        });
        let (ws, _) = connect_async(format!("ws://{address}/")).await.unwrap();
        let mut client = Client {
            ws,
            key: BOOT_KEY.to_vec(),
            task: 0,
            uin: "123".into(),
            device: "device".into(),
        };
        let (audio_tx, audio_rx) = mpsc::channel(10);
        // Queue the second phrase before the first has even been finalized.
        for _ in 0..2 {
            audio_tx
                .send(segment::AudioEvent::Frame(vec![0; 640]))
                .await
                .unwrap();
            audio_tx.send(segment::AudioEvent::End).await.unwrap();
        }
        drop(audio_tx);
        let (events, mut received) = mpsc::channel(10);
        process_segments(&mut client, audio_rx, &events)
            .await
            .unwrap();
        drop(events);
        let mut finals = Vec::new();
        while let Some(event) = received.recv().await {
            if let AsrEvent::ResultGenerated {
                text,
                is_final: true,
            } = event
            {
                finals.push(text);
            }
        }
        assert_eq!(finals, ["第一句。", "第二句。"]);
        server.await.unwrap();
    }

    #[test]
    fn identity_is_private_and_compatible() {
        use std::os::unix::fs::PermissionsExt;
        let preferred = std::path::PathBuf::from("/tmp/opencode");
        let temp_dir = if preferred.is_dir() {
            preferred
        } else {
            std::env::temp_dir()
        };
        let path = temp_dir.join(format!("wetype-identity-test-{}.json", Uuid::new_v4()));
        let identity = Identity {
            device: "test-device".into(),
            uin: 123.into(),
        };
        save_identity(&path, &identity).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let loaded: Identity = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.uin.as_u64(), Some(123));
        assert_eq!(loaded.device, "test-device");
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn voice_roundtrip_over_websocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let msg = ws.next().await.unwrap().unwrap().into_data();
            let outer = fields(&msg).unwrap();
            assert_eq!(get_number(&outer, 3), Some(1));
            let request = fields(get_bytes(&outer, 5).unwrap()).unwrap();
            assert_eq!(get_bytes(&request, 3), Some(b"/api_v2".as_slice()));
            let enc = get_bytes(&request, 6).unwrap();
            let compressed = decrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, enc).unwrap();
            let plain = snap::raw::Decoder::new()
                .decompress_vec(&compressed)
                .unwrap();
            let inner = fields(&plain).unwrap();
            let voice = fields(get_bytes(&inner, 1).unwrap()).unwrap();
            assert_eq!(get_bytes(&voice, 2), Some(b"voice-id".as_slice()));
            assert_eq!(get_number(&voice, 7), Some(3));
            assert_eq!(get_number(&voice, 11), Some(100));
            assert_eq!(get_number(&voice, 6), Some(1));
            assert_eq!(get_bytes(&voice, 4), Some(b"opus".as_slice()));
            let mut text = Vec::new();
            bytes(&mut text, 4, "临时");
            bytes(&mut text, 14, "最终文本");
            let mut inner = Vec::new();
            bytes(&mut inner, 1, text);
            let compressed = snap::raw::Encoder::new().compress_vec(&inner).unwrap();
            let enc = encrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &compressed).unwrap();
            let mut response = Vec::new();
            number(&mut response, 2, 200);
            let mut header = Vec::new();
            bytes(&mut header, 1, "Kb-CompressionType");
            bytes(&mut header, 2, "2");
            bytes(&mut response, 4, header);
            bytes(&mut response, 5, enc);
            let mut outer = Vec::new();
            bytes(&mut outer, 4, response);
            ws.send(Message::Binary(outer)).await.unwrap();
        });
        let (ws, _) = connect_async(format!("ws://{address}/")).await.unwrap();
        let mut client = Client {
            ws,
            key: BOOT_KEY.to_vec(),
            task: 0,
            uin: "123".into(),
            device: "device".into(),
        };
        assert_eq!(
            client
                .voice("voice-id", b"opus", 3, 100, true)
                .await
                .unwrap(),
            ("临时".into(), "最终文本".into())
        );
        server.await.unwrap();
    }

    #[test]
    fn shared_key_matches_reference_curve() {
        let group = EcGroup::from_curve_name(Nid::SECP128R1).unwrap();
        let private = BigNum::from_u32(1).unwrap();
        let public = "04161FF7528B899B2D0C28607CA52C5B86CF5AC8395BAFEB13C02DA292DDED7A83";
        assert_eq!(
            derive_key(&group, &private, public).unwrap(),
            b"161FF7528B899B2D0C28607CA52C5B86"
        );
        assert!(derive_key(&group, &private, &format!("04{}", "00".repeat(32))).is_err());
    }

    #[test]
    fn opus_framing_and_total() {
        let mut framer = AudioFramer::new().unwrap();
        framer.encode_frame(&[0; 640]).unwrap();
        let first = framer.take();
        assert!(first.starts_with(b"#!OPUS_RAW_V1\x02\x01\x00"));
        let length = u16::from_le_bytes([first[16], first[17]]) as usize;
        assert_eq!(first.len(), 18 + length);
        framer.encode_frame(&[0; 640]).unwrap();
        let second = framer.take();
        assert_eq!(framer.seq, 2);
        assert_eq!(framer.total as usize, first.len() + second.len());
        let mut decoder = opus::Decoder::new(16000, opus::Channels::Mono).unwrap();
        assert_eq!(
            decoder.decode(&first[18..], &mut [0; 320], false).unwrap(),
            320
        );
    }

    #[test]
    fn encryption_padding_and_snappy() {
        let plain = b"protobuf payload";
        let compressed = snap::raw::Encoder::new().compress_vec(plain).unwrap();
        let enc = encrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &compressed).unwrap();
        let dec = decrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &enc).unwrap();
        assert_eq!(
            snap::raw::Decoder::new().decompress_vec(&dec).unwrap(),
            plain
        );
        assert!(decrypt(Cipher::aes_256_ecb(), BOOT_KEY, None, &[1, 2]).is_err());
    }
}
