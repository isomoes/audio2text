use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const WS_URL: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub action: Option<String>,
    pub task_id: String,
    pub streaming: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Word {
    pub begin_time: i64,
    pub end_time: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub punctuation: Option<String>,
    #[serde(default)]
    pub fixed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcription {
    pub sentence_id: i32,
    pub begin_time: i64,
    pub end_time: i64,
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub sentence_end: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sentence {
    pub sentence_id: i32,
    pub begin_time: i64,
    pub end_time: Option<i64>,
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub sentence_end: bool,
    #[serde(default)]
    pub heartbeat: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Translation {
    pub sentence_id: i32,
    pub begin_time: i64,
    pub end_time: i64,
    pub text: String,
    pub lang: String,
    #[serde(default)]
    pub pre_end_failed: bool,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub sentence_end: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentence: Option<Sentence>,
    #[serde(default)]
    pub translations: Vec<Translation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcription: Option<Transcription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocabulary_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_hints: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Input {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Parameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Input>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub header: Header,
    pub payload: Payload,
}

#[derive(Debug, Clone)]
pub enum AsrEvent {
    TaskStarted,
    ResultGenerated { text: String, is_final: bool },
    TaskFinished,
    TaskFailed { error: String },
}

pub struct AsrClient {
    api_key: String,
    task_id: Option<String>,
}

impl AsrClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            task_id: None,
        }
    }

    fn generate_run_task_cmd(&mut self) -> Event {
        let task_id = Uuid::new_v4().to_string();
        self.task_id = Some(task_id.clone());

        Event {
            header: Header {
                action: Some("run-task".to_string()),
                task_id,
                streaming: Some("duplex".to_string()),
                event: None,
                error_code: None,
                error_message: None,
                attributes: None,
            },
            payload: Payload {
                task_group: Some("audio".to_string()),
                task: Some("asr".to_string()),
                function: Some("recognition".to_string()),
                model: Some("qwen-audio-3.0-asr-flash-streaming".to_string()),
                parameters: Some(Parameters {
                    format: Some("pcm".to_string()),
                    sample_rate: Some(16000),
                    vocabulary_id: None,
                    language_hints: Some(vec!["zh".to_string()]),
                }),
                input: Some(Input {}),
                output: None,
            },
        }
    }

    fn generate_finish_task_cmd(&self) -> Option<Event> {
        self.task_id.as_ref().map(|task_id| Event {
            header: Header {
                action: Some("finish-task".to_string()),
                task_id: task_id.clone(),
                streaming: Some("duplex".to_string()),
                event: None,
                error_code: None,
                error_message: None,
                attributes: None,
            },
            payload: Payload {
                task_group: None,
                task: None,
                function: None,
                model: None,
                parameters: None,
                input: Some(Input {}),
                output: None,
            },
        })
    }

    pub async fn start_recognition(
        &mut self,
        mut audio_rx: mpsc::Receiver<Vec<u8>>,
        event_tx: mpsc::Sender<AsrEvent>,
    ) -> Result<()> {
        let url = url::Url::parse(WS_URL)?;

        let request = http::Request::builder()
            .uri(WS_URL)
            .header("Authorization", format!("bearer {}", self.api_key))
            .header("Host", url.host_str().unwrap_or("dashscope.aliyuncs.com"))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())?;

        let (ws_stream, _) = connect_async(request)
            .await
            .context("Failed to connect to WebSocket")?;

        info!("Connected to DashScope WebSocket");

        let (mut write, mut read) = ws_stream.split();

        // Send run-task command
        let run_task_cmd = self.generate_run_task_cmd();
        let run_task_json = serde_json::to_string(&run_task_cmd)?;
        debug!("Sending run-task: {}", run_task_json);
        write.send(Message::Text(run_task_json)).await?;

        // Wait for task-started event
        let mut task_started = false;
        while !task_started {
            if let Some(msg) = read.next().await {
                match msg? {
                    Message::Text(text) => {
                        let event: Event = serde_json::from_str(&text)?;
                        if let Some(event_type) = &event.header.event {
                            if event_type == "task-started" {
                                info!("Task started");
                                task_started = true;
                                event_tx.send(AsrEvent::TaskStarted).await?;
                            } else if event_type == "task-failed" {
                                let error = event
                                    .header
                                    .error_message
                                    .unwrap_or_else(|| "Unknown error".to_string());
                                error!("Task failed: {}", error);
                                event_tx
                                    .send(AsrEvent::TaskFailed {
                                        error: error.clone(),
                                    })
                                    .await?;
                                return Err(anyhow::anyhow!("Task failed: {}", error));
                            }
                        }
                    }
                    Message::Close(_) => {
                        return Err(anyhow::anyhow!("Connection closed before task started"));
                    }
                    _ => {}
                }
            }
        }

        let event_tx_clone = event_tx.clone();
        let _task_id = self.task_id.clone();

        // Spawn task to handle incoming messages
        let read_handle = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(event) = serde_json::from_str::<Event>(&text) {
                            if let Some(event_type) = &event.header.event {
                                match event_type.as_str() {
                                    "result-generated" => {
                                        if let Some(output) = &event.payload.output {
                                            if let Some(sentence) = &output.sentence {
                                                // Ignore heartbeat packets; they contain no text.
                                                if sentence.heartbeat || sentence.text.is_empty() {
                                                    continue;
                                                }
                                                let is_final = sentence.sentence_end;
                                                let text = sentence.text.clone();
                                                debug!(
                                                    "Transcription: {} (final: {})",
                                                    text, is_final
                                                );
                                                let _ = event_tx_clone
                                                    .send(AsrEvent::ResultGenerated {
                                                        text,
                                                        is_final,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                    "task-finished" => {
                                        info!("Task finished");
                                        let _ = event_tx_clone.send(AsrEvent::TaskFinished).await;
                                        break;
                                    }
                                    "task-failed" => {
                                        let error = event
                                            .header
                                            .error_message
                                            .unwrap_or_else(|| "Unknown error".to_string());
                                        error!("Task failed: {}", error);
                                        let _ = event_tx_clone
                                            .send(AsrEvent::TaskFailed { error })
                                            .await;
                                        break;
                                    }
                                    _ => {
                                        warn!("Unknown event: {}", event_type);
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket closed");
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Send audio data
        while let Some(audio_data) = audio_rx.recv().await {
            if let Err(e) = write.send(Message::Binary(audio_data)).await {
                error!("Failed to send audio: {}", e);
                break;
            }
        }

        // Send finish-task command
        if let Some(finish_cmd) = self.generate_finish_task_cmd() {
            let finish_json = serde_json::to_string(&finish_cmd)?;
            debug!("Sending finish-task: {}", finish_json);
            write.send(Message::Text(finish_json)).await?;
        }

        // Wait for read task to complete
        let _ = read_handle.await;

        Ok(())
    }
}
