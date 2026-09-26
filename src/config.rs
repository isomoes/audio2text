use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const DEFAULT_LANGUAGE_HINTS: &[&str] = &["zh", "en"];
const DEFAULT_VAD_MODEL: &str = "far_field_meeting_16k";
const DEFAULT_HOTWORD_WEIGHT: u32 = 4;
const SUPER_HOTWORD_WEIGHT: u32 = 50;
const MAX_HOTWORDS: usize = 2000;
const MAX_SUPER_HOTWORDS: usize = 50;
const MAX_LANGUAGE_HINTS: usize = 4;

#[derive(Debug, Clone)]
pub struct Config {
    pub language_hints: Vec<String>,
    /// Instant hotwords: text -> weight (1-5, or 50 for a "super" hotword).
    pub hotwords: BTreeMap<String, u32>,
    pub vad_model: String,
    pub speech_noise_threshold: Option<f32>,
    /// VAD silence (ms) that ends a sentence; `None` uses the server default (1300).
    pub max_sentence_silence: Option<u32>,
    /// Split sentences semantically instead of by VAD silence; `None` uses the server default (off).
    pub semantic_punctuation: Option<bool>,
    pub dump_wav: Option<PathBuf>,
}

impl Config {
    /// Environment variables:
    /// - `AUDIO2TEXT_LANGUAGE_HINTS`: comma-separated codes (default `zh,en`, max 4)
    /// - `AUDIO2TEXT_HOTWORDS`: hotword file path (default `~/.config/audio2text/hotwords.txt` if it exists)
    /// - `AUDIO2TEXT_VAD_MODEL`: `far_field_meeting_16k` (default) or `near_meeting_16k`
    /// - `AUDIO2TEXT_SPEECH_NOISE_THRESHOLD`: VAD sensitivity in [-1.0, 1.0]; lower keeps more speech
    /// - `AUDIO2TEXT_MAX_SENTENCE_SILENCE`: silence in ms (200-6000) that ends a sentence
    /// - `AUDIO2TEXT_SEMANTIC_PUNCTUATION`: `true`/`false`, split sentences semantically instead of by silence
    /// - `AUDIO2TEXT_DUMP_WAV`: if set, write the audio sent to the ASR service to this WAV file
    pub fn from_env() -> Result<Self> {
        let language_hints =
            parse_language_hints(std::env::var("AUDIO2TEXT_LANGUAGE_HINTS").ok().as_deref());

        let hotwords = match hotwords_path() {
            Some(path) => {
                let hotwords = load_hotwords(&path)?;
                info!("Loaded {} hotwords from {}", hotwords.len(), path.display());
                hotwords
            }
            None => BTreeMap::new(),
        };

        let vad_model =
            non_empty_env("AUDIO2TEXT_VAD_MODEL").unwrap_or_else(|| DEFAULT_VAD_MODEL.to_string());

        let speech_noise_threshold = non_empty_env("AUDIO2TEXT_SPEECH_NOISE_THRESHOLD")
            .map(|v| parse_speech_noise_threshold(&v))
            .transpose()?;

        let max_sentence_silence = non_empty_env("AUDIO2TEXT_MAX_SENTENCE_SILENCE")
            .map(|v| parse_max_sentence_silence(&v))
            .transpose()?;

        let semantic_punctuation = non_empty_env("AUDIO2TEXT_SEMANTIC_PUNCTUATION")
            .map(|v| parse_bool("AUDIO2TEXT_SEMANTIC_PUNCTUATION", &v))
            .transpose()?;

        let dump_wav = non_empty_env("AUDIO2TEXT_DUMP_WAV").map(PathBuf::from);

        Ok(Self {
            language_hints,
            hotwords,
            vad_model,
            speech_noise_threshold,
            max_sentence_silence,
            semantic_punctuation,
            dump_wav,
        })
    }
}

fn parse_max_sentence_silence(value: &str) -> Result<u32> {
    let ms: u32 = value
        .parse()
        .with_context(|| format!("AUDIO2TEXT_MAX_SENTENCE_SILENCE: invalid integer {value:?}"))?;
    if !(200..=6000).contains(&ms) {
        bail!("AUDIO2TEXT_MAX_SENTENCE_SILENCE must be in [200, 6000] ms, got {ms}");
    }
    Ok(ms)
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("{key}: expected true/false, got {value:?}"),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_speech_noise_threshold(value: &str) -> Result<f32> {
    let threshold: f32 = value
        .parse()
        .with_context(|| format!("AUDIO2TEXT_SPEECH_NOISE_THRESHOLD: invalid number {value:?}"))?;
    if !(-1.0..=1.0).contains(&threshold) {
        bail!("AUDIO2TEXT_SPEECH_NOISE_THRESHOLD must be in [-1.0, 1.0], got {threshold}");
    }
    Ok(threshold)
}

fn parse_language_hints(value: Option<&str>) -> Vec<String> {
    let mut hints: Vec<String> = match value {
        Some(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => DEFAULT_LANGUAGE_HINTS
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };
    if hints.len() > MAX_LANGUAGE_HINTS {
        warn!(
            "Only the first {} language hints are used by the model",
            MAX_LANGUAGE_HINTS
        );
        hints.truncate(MAX_LANGUAGE_HINTS);
    }
    hints
}

fn hotwords_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("AUDIO2TEXT_HOTWORDS") {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    let default = config_home.join("audio2text").join("hotwords.txt");
    default.exists().then_some(default)
}

fn load_hotwords(path: &Path) -> Result<BTreeMap<String, u32>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read hotwords file {}", path.display()))?;
    parse_hotwords(&content).with_context(|| format!("Invalid hotwords file {}", path.display()))
}

/// One hotword per line, optionally followed by a weight (1-5, or 50):
///
/// ```text
/// # comment
/// 通义千问 5
/// WebSocket
/// John Smith 3
/// ```
fn parse_hotwords(content: &str) -> Result<BTreeMap<String, u32>> {
    let mut hotwords = BTreeMap::new();

    for (lineno, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (word, weight) = match line.rsplit_once(char::is_whitespace) {
            Some((word, last)) if last.parse::<u32>().is_ok() => {
                (word.trim(), last.parse::<u32>().unwrap())
            }
            _ => (line, DEFAULT_HOTWORD_WEIGHT),
        };

        if !((1..=5).contains(&weight) || weight == SUPER_HOTWORD_WEIGHT) {
            bail!(
                "line {}: weight {} must be 1-5 or {}",
                lineno + 1,
                weight,
                SUPER_HOTWORD_WEIGHT
            );
        }
        if word.is_empty() {
            bail!("line {}: empty hotword", lineno + 1);
        }
        hotwords.insert(word.to_string(), weight);
    }

    if hotwords.len() > MAX_HOTWORDS {
        bail!(
            "{} hotwords exceeds the limit of {}",
            hotwords.len(),
            MAX_HOTWORDS
        );
    }
    let super_count = hotwords
        .values()
        .filter(|&&w| w == SUPER_HOTWORD_WEIGHT)
        .count();
    if super_count > MAX_SUPER_HOTWORDS {
        bail!(
            "{} super hotwords (weight {}) exceeds the limit of {}",
            super_count,
            SUPER_HOTWORD_WEIGHT,
            MAX_SUPER_HOTWORDS
        );
    }

    Ok(hotwords)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotwords_with_and_without_weights() {
        let map = parse_hotwords("# c\n\n通义千问 5\nWebSocket\nJohn Smith 3\nGPU 50\n").unwrap();
        assert_eq!(map.get("通义千问"), Some(&5));
        assert_eq!(map.get("WebSocket"), Some(&DEFAULT_HOTWORD_WEIGHT));
        assert_eq!(map.get("John Smith"), Some(&3));
        assert_eq!(map.get("GPU"), Some(&50));
    }

    #[test]
    fn rejects_invalid_weight() {
        assert!(parse_hotwords("foo 7\n").is_err());
    }

    #[test]
    fn max_sentence_silence_range() {
        assert_eq!(parse_max_sentence_silence("2000").unwrap(), 2000);
        assert!(parse_max_sentence_silence("100").is_err());
        assert!(parse_max_sentence_silence("7000").is_err());
        assert!(parse_max_sentence_silence("abc").is_err());
    }

    #[test]
    fn parses_bools() {
        assert!(parse_bool("K", "true").unwrap());
        assert!(parse_bool("K", "ON").unwrap());
        assert!(!parse_bool("K", "0").unwrap());
        assert!(parse_bool("K", "maybe").is_err());
    }

    #[test]
    fn speech_noise_threshold_range() {
        assert_eq!(parse_speech_noise_threshold("-0.2").unwrap(), -0.2);
        assert!(parse_speech_noise_threshold("1.5").is_err());
        assert!(parse_speech_noise_threshold("abc").is_err());
    }

    #[test]
    fn language_hints_default_and_truncate() {
        assert_eq!(parse_language_hints(None), vec!["zh", "en"]);
        assert_eq!(parse_language_hints(Some("zh, en,ja,ko,fr")).len(), 4);
    }
}
