use anyhow::{Context, Result};
use std::path::Path;
use tokio::sync::mpsc;
use tracing::{error, info};

use super::TARGET_SAMPLE_RATE;

/// Forward PCM chunks from `audio_rx` to a new receiver while also writing them
/// to a 16 kHz mono 16-bit WAV file, so the exact audio sent to the ASR service
/// can be inspected.
pub fn tee_to_wav(
    mut audio_rx: mpsc::Receiver<Vec<u8>>,
    path: &Path,
) -> Result<mpsc::Receiver<Vec<u8>>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("Failed to create WAV dump at {}", path.display()))?;
    info!("Dumping sent audio to {}", path.display());

    let (tx, rx) = mpsc::channel::<Vec<u8>>(100);
    tokio::spawn(async move {
        let mut write_ok = true;
        while let Some(chunk) = audio_rx.recv().await {
            if write_ok {
                let result = chunk
                    .chunks_exact(2)
                    .try_for_each(|b| writer.write_sample(i16::from_le_bytes([b[0], b[1]])))
                    // Flush every chunk so the header stays valid if the process is killed.
                    .and_then(|_| writer.flush());
                if let Err(e) = result {
                    error!("Failed to write WAV dump, disabling: {}", e);
                    write_ok = false;
                }
            }
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
        let _ = writer.finalize();
    });

    Ok(rx)
}
