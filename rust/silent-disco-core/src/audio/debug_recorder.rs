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
/// Largest data chunk representable by canonical RIFF/WAVE, accounting for
/// the 36 bytes between the RIFF size field and the PCM payload.
const MAX_WAV_DATA_BYTES: u32 = u32::MAX - (WAV_HEADER_BYTES - 8);
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
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "debug PCM recorder is already finished",
            ));
        }

        // Validate the final RIFF size before writing anything. Saturating the
        // counter after the bytes have already reached disk would create a WAV
        // whose header cannot describe its payload and make a diagnostic
        // capture silently misleading.
        let append_bytes = samples
            .len()
            .checked_mul(core::mem::size_of::<i16>())
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "debug PCM append is too large for a WAV data chunk",
                )
            })?;
        let new_data_bytes = self.data_bytes.checked_add(append_bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "debug PCM capture exceeds the WAV data-size limit",
            )
        })?;
        if new_data_bytes > MAX_WAV_DATA_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "debug PCM capture exceeds the RIFF size limit",
            ));
        }

        for &sample in samples {
            self.writer.write_all(&sample.to_le_bytes())?;
        }
        self.data_bytes = new_data_bytes;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "silent-disco-debug-recorder-{name}-{}-{:?}.wav",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn append_after_finish_is_an_error_instead_of_a_silent_drop() {
        let path = recorder_path("after-finish");
        let mut recorder = DebugPcmRecorder::create(&path, 48_000, 2).expect("recorder");
        recorder.append(&[1, -1]).expect("initial append");
        recorder.finish(48_000).expect("finish");

        let error = recorder
            .append(&[2, -2])
            .expect_err("post-finish audio must not be silently discarded");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(recorder.data_bytes(), 4);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn append_refuses_riff_overflow_before_writing_any_sample_bytes() {
        let path = recorder_path("riff-overflow");
        let mut recorder = DebugPcmRecorder::create(&path, 48_000, 2).expect("recorder");
        // Private state is intentionally set at the boundary so this test can
        // exercise a multi-gigabyte file limit without allocating or writing
        // a multi-gigabyte fixture.
        recorder.data_bytes = MAX_WAV_DATA_BYTES - 1;
        recorder.writer.flush().expect("flush header");
        let before_len = std::fs::metadata(&path).expect("metadata").len();

        let error = recorder
            .append(&[7])
            .expect_err("an unrepresentable RIFF payload must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(recorder.data_bytes(), MAX_WAV_DATA_BYTES - 1);
        recorder.writer.flush().expect("flush");
        assert_eq!(
            std::fs::metadata(&path).expect("metadata").len(),
            before_len,
            "overflow rejection must happen before sample bytes are written"
        );
        std::fs::remove_file(path).ok();
    }
}
