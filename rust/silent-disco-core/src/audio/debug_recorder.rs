//! Optional capture of exactly the PCM a stream released toward its render
//! ring, written as a playable WAV for offline analysis.
//!
//! Every audio defect fixed in this pipeline was identified by comparing such
//! a recording against the diagnostics counters — sample-level discontinuities
//! and silence gaps are objective where a description of what playback
//! "sounded like" is not. This is diagnostic instrumentation, off unless a
//! path is configured.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// Bytes in a canonical PCM WAV header, before any sample data.
const WAV_HEADER_BYTES: u32 = 44;
const PCM_FORMAT_TAG: u16 = 1;
const BITS_PER_SAMPLE: u16 = 16;

/// Writes interleaved PCM16 samples to a WAV file, patching the header's
/// length fields when the stream ends.
#[derive(Debug)]
pub struct DebugPcmRecorder {
    writer: BufWriter<File>,
    channels: u16,
    data_bytes: u32,
    finished: bool,
}

impl DebugPcmRecorder {
    /// Creates a recorder at `path`, reserving the header to be patched by
    /// [`Self::finish`].
    ///
    /// # Errors
    ///
    /// Returns any I/O error from creating the file or writing its header.
    pub fn create(path: impl AsRef<Path>, sample_rate: u32, channels: u16) -> io::Result<Self> {
        let mut writer = BufWriter::new(File::create(path)?);
        write_header(&mut writer, sample_rate, channels, 0)?;
        Ok(Self {
            writer,
            channels,
            data_bytes: 0,
            finished: false,
        })
    }

    /// Appends one frame's interleaved samples.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from writing the samples.
    pub fn append(&mut self, samples: &[i16]) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        for &sample in samples {
            self.writer.write_all(&sample.to_le_bytes())?;
        }
        self.data_bytes = self
            .data_bytes
            .saturating_add(u32::try_from(samples.len() * 2).unwrap_or(u32::MAX));
        Ok(())
    }

    /// Patches the header with the final lengths and flushes. Idempotent; a
    /// recording that is never finished still holds valid samples but reports
    /// a zero length to players.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from seeking, rewriting the header, or flushing.
    pub fn finish(&mut self, sample_rate: u32) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.writer.flush()?;
        self.writer.seek(SeekFrom::Start(0))?;
        write_header(
            &mut self.writer,
            sample_rate,
            self.channels,
            self.data_bytes,
        )?;
        self.writer.flush()
    }

    /// Bytes of sample data written so far.
    #[must_use]
    pub const fn data_bytes(&self) -> u32 {
        self.data_bytes
    }
}

fn write_header(
    writer: &mut impl Write,
    sample_rate: u32,
    channels: u16,
    data_bytes: u32,
) -> io::Result<()> {
    let block_align = channels * BITS_PER_SAMPLE / 8;
    let byte_rate = sample_rate * u32::from(block_align);

    writer.write_all(b"RIFF")?;
    writer.write_all(&(WAV_HEADER_BYTES - 8 + data_bytes).to_le_bytes())?;
    writer.write_all(b"WAVE")?;
    writer.write_all(b"fmt ")?;
    writer.write_all(&16_u32.to_le_bytes())?;
    writer.write_all(&PCM_FORMAT_TAG.to_le_bytes())?;
    writer.write_all(&channels.to_le_bytes())?;
    writer.write_all(&sample_rate.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&block_align.to_le_bytes())?;
    writer.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())
}
