//! Local energy-based pause detection, independent of network round trips.
use anyhow::{bail, Result};
use std::collections::VecDeque;
use tokio::sync::mpsc;

const FRAME_BYTES: usize = 640; // 20ms, 16kHz mono s16le
const PRE_ROLL_FRAMES: usize = 10; // Keep 200ms before the first loud frame.
const MAX_SEGMENT_FRAMES: usize = 1500; // Finalize continuous speech every 30s.

#[derive(Debug)]
pub(super) enum AudioEvent {
    Frame(Vec<u8>),
    End,
}

struct Segmenter {
    silence_frames: usize,
    threshold: f32,
    active: bool,
    quiet: usize,
    length: usize,
    pre_roll: VecDeque<Vec<u8>>,
}

impl Segmenter {
    fn new(silence_ms: u32, threshold: f32) -> Self {
        Self {
            silence_frames: silence_ms.div_ceil(20) as usize,
            threshold,
            active: false,
            quiet: 0,
            length: 0,
            pre_roll: VecDeque::new(),
        }
    }

    fn frame(&mut self, frame: Vec<u8>) -> Vec<AudioEvent> {
        let energy: f32 = frame
            .chunks_exact(2)
            .map(|b| {
                let sample = f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0;
                sample * sample
            })
            .sum();
        let voice = (energy / 320.0).sqrt() >= self.threshold;
        let mut events = Vec::new();
        if !self.active {
            if !voice {
                self.pre_roll.push_back(frame);
                if self.pre_roll.len() > PRE_ROLL_FRAMES {
                    self.pre_roll.pop_front();
                }
                return events;
            }
            self.active = true;
            self.length = self.pre_roll.len();
            events.extend(self.pre_roll.drain(..).map(AudioEvent::Frame));
        }
        self.quiet = if voice { 0 } else { self.quiet + 1 };
        self.length += 1;
        events.push(AudioEvent::Frame(frame));
        if self.quiet >= self.silence_frames || self.length >= MAX_SEGMENT_FRAMES {
            events.push(AudioEvent::End);
            self.active = false;
            self.quiet = 0;
            self.length = 0;
        }
        events
    }
}

pub(super) async fn produce(
    mut audio: mpsc::Receiver<Vec<u8>>,
    output: mpsc::Sender<AudioEvent>,
    silence_ms: u32,
    threshold: f32,
) -> Result<()> {
    let mut segmenter = Segmenter::new(silence_ms, threshold);
    let mut pcm = Vec::new();
    while let Some(chunk) = audio.recv().await {
        pcm.extend(chunk);
        while pcm.len() >= FRAME_BYTES {
            let frame = pcm.drain(..FRAME_BYTES).collect();
            for event in segmenter.frame(frame) {
                output.send(event).await?;
            }
        }
    }
    if pcm.len() % 2 != 0 {
        bail!("WeType PCM ends with an incomplete 16-bit sample");
    }
    if !pcm.is_empty() {
        pcm.resize(FRAME_BYTES, 0);
        for event in segmenter.frame(pcm) {
            output.send(event).await?;
        }
    }
    if segmenter.active {
        output.send(AudioEvent::End).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sample: i16) -> Vec<u8> {
        sample.to_le_bytes().repeat(320)
    }

    #[test]
    fn segments_on_pause_and_keeps_pre_roll() {
        let mut segmenter = Segmenter::new(1000, 0.01);
        for _ in 0..20 {
            assert!(segmenter.frame(frame(0)).is_empty());
        }
        let first = segmenter.frame(frame(1000));
        assert_eq!(first.len(), PRE_ROLL_FRAMES + 1);
        for _ in 0..49 {
            assert!(!segmenter
                .frame(frame(0))
                .iter()
                .any(|e| matches!(e, AudioEvent::End)));
        }
        assert!(matches!(
            segmenter.frame(frame(0)).last(),
            Some(AudioEvent::End)
        ));
        assert_eq!(segmenter.frame(frame(1000)).len(), 1);
    }

    #[test]
    fn short_pause_does_not_end_speech() {
        let mut segmenter = Segmenter::new(1000, 0.01);
        segmenter.frame(frame(1000));
        for _ in 0..40 {
            segmenter.frame(frame(0));
        }
        segmenter.frame(frame(1000));
        for _ in 0..40 {
            assert!(!segmenter
                .frame(frame(0))
                .iter()
                .any(|e| matches!(e, AudioEvent::End)));
        }
    }

    #[test]
    fn continuous_speech_is_bounded() {
        let mut segmenter = Segmenter::new(1000, 0.01);
        for _ in 0..MAX_SEGMENT_FRAMES - 1 {
            segmenter.frame(frame(1000));
        }
        assert!(matches!(
            segmenter.frame(frame(1000)).last(),
            Some(AudioEvent::End)
        ));
        assert_eq!(segmenter.frame(frame(1000)).len(), 1);
    }

    #[tokio::test]
    async fn buffers_next_segment_while_consumer_waits_and_flushes_eof() {
        let (audio_tx, audio_rx) = mpsc::channel(10);
        let (output_tx, mut output_rx) = mpsc::channel(100);
        audio_tx
            .send(
                [
                    frame(1000),
                    frame(0).repeat(10),
                    frame(1000),
                    frame(1000)[..320].to_vec(),
                ]
                .concat(),
            )
            .await
            .unwrap();
        drop(audio_tx);
        produce(audio_rx, output_tx, 200, 0.01).await.unwrap();
        let mut ends = 0;
        let mut frames = 0;
        while let Some(event) = output_rx.recv().await {
            match event {
                AudioEvent::End => ends += 1,
                AudioEvent::Frame(_) => frames += 1,
            }
        }
        assert_eq!(ends, 2);
        assert_eq!(frames, 13);
    }

    #[tokio::test]
    async fn shutdown_finalizes_last_speech_and_silence_is_not_submitted() {
        for (sample, expected) in [(1000, 1), (0, 0)] {
            let (tx, rx) = mpsc::channel(2);
            let (out, mut events) = mpsc::channel(20);
            tx.send(frame(sample)).await.unwrap();
            drop(tx);
            produce(rx, out, 1000, 0.01).await.unwrap();
            let mut ends = 0;
            while let Some(event) = events.recv().await {
                if matches!(event, AudioEvent::End) {
                    ends += 1;
                }
            }
            assert_eq!(ends, expected);
        }
    }
}
