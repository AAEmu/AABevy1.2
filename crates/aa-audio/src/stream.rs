//! Streamed playback of long Vorbis subsounds (music): the mixer pulls decoded chunks as it needs
//! them, so only the compressed bytes of the track are in memory.

use kira::Frame;
#[cfg(not(target_arch = "wasm32"))]
use kira::sound::streaming::Decoder;

use crate::fsb5::Error;
use crate::vorbis::Stream;

/// [`Decoder`] over an FSB Vorbis stream.
pub struct StreamDecoder {
    stream: Stream,
    emitted: usize,
}

impl StreamDecoder {
    pub fn new(stream: Stream) -> StreamDecoder {
        StreamDecoder { stream, emitted: 0 }
    }

    /// The next chunk of frames (never empty).
    fn decode_chunk(&mut self) -> Result<Vec<Frame>, Error> {
        let total = self.stream.frames();
        let channels = self.stream.channels().max(1) as usize;
        loop {
            match self.stream.next_block()? {
                Some(block) if block.is_empty() => continue,
                Some(block) => {
                    let scale = 1.0 / 32768.0;
                    let frames: Vec<Frame> = block
                        .chunks_exact(channels)
                        .take(total.saturating_sub(self.emitted))
                        .map(|f| {
                            let l = f[0] as f32 * scale;
                            Frame { left: l, right: if channels > 1 { f[1] as f32 * scale } else { l } }
                        })
                        .collect();
                    self.emitted += frames.len();
                    if frames.is_empty() {
                        continue;
                    }
                    return Ok(frames);
                }
                None => {
                    // The mixer keeps asking until it has `total` frames: pad with silence if the
                    // stream ended early, and never hand back an empty chunk.
                    let pad = total.saturating_sub(self.emitted).clamp(1, 4096);
                    self.emitted += pad;
                    return Ok(vec![Frame::ZERO; pad]);
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Decoder for StreamDecoder {
    type Error = Error;

    fn sample_rate(&self) -> u32 {
        self.stream.rate()
    }

    fn num_frames(&self) -> usize {
        self.stream.frames()
    }

    fn decode(&mut self) -> Result<Vec<Frame>, Error> {
        self.decode_chunk()
    }

    fn seek(&mut self, _index: usize) -> Result<usize, Error> {
        // Restart from the beginning; the mixer decodes forward to the frame it wants.
        self.stream.rewind();
        self.emitted = 0;
        Ok(0)
    }
}

/// The whole track as frames: kira cannot stream in the browser, so music there plays as a static sound.
#[cfg(target_arch = "wasm32")]
pub fn decode_all(stream: Stream) -> Result<(u32, std::sync::Arc<[Frame]>), Error> {
    let rate = stream.rate();
    let total = stream.frames();
    let mut decoder = StreamDecoder::new(stream);
    let mut frames = Vec::with_capacity(total);
    while frames.len() < total {
        frames.extend(decoder.decode_chunk()?);
    }
    frames.truncate(total);
    Ok((rate, frames.into()))
}
