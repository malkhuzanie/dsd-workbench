use std::io::{self, Read, Seek, SeekFrom};

use super::{Container, DsdSource, PAD_BYTE, StreamInfo};
use crate::error::Result;

/// DSF channel-type ids for common channel counts (best effort; verify for multichannel).
fn channel_type(channels: u32) -> u32 {
    match channels {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        6 => 7,
        _ => 0,
    }
}

/// Reads per-channel byte ranges from the byte-interleaved audio of a DSDIFF file.
pub struct DffSource<R> {
    src: R,
    info: StreamInfo,
    audio_offset: u64,
    channels: usize,
    /// Audio bytes per channel.
    valid: u64,
    scratch: Vec<u8>,
}

impl<R: Read + Seek> DffSource<R> {
    pub fn new(mut src: R) -> Result<Self> {
        let d = dsdiff::read_info(&mut src)?;
        Ok(Self {
            src,
            info: StreamInfo {
                container: Container::Dsdiff,
                sample_rate: d.sample_rate,
                channels: d.channel_count,
                channel_type: channel_type(d.channel_count),
                sample_count: d.sample_count,
                msb_first: true,
            },
            audio_offset: d.audio_offset,
            channels: d.channel_count as usize,
            valid: d.audio_len / u64::from(d.channel_count),
            scratch: Vec::new(),
        })
    }
}

impl<R: Read + Seek + Send> DsdSource for DffSource<R> {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, first: i64, len: usize) -> io::Result<Vec<Vec<u8>>> {
        let mut out = vec![vec![PAD_BYTE; len]; self.channels];
        let end = first.saturating_add(len as i64);
        let lo = first.max(0) as u64;
        let hi = (end.max(0) as u64).min(self.valid);
        if lo >= hi {
            return Ok(out);
        }

        let count = (hi - lo) as usize;
        self.src.seek(SeekFrom::Start(
            self.audio_offset
                .saturating_add(lo.saturating_mul(self.channels as u64)),
        ))?;
        self.scratch.resize(count * self.channels, 0);
        self.src.read_exact(&mut self.scratch)?;

        let dst = (lo as i64 - first) as usize;
        for (i, frame) in self.scratch.chunks_exact(self.channels).enumerate() {
            for (channel, byte) in frame.iter().enumerate() {
                if let Some(slot) = out.get_mut(channel).and_then(|c| c.get_mut(dst + i)) {
                    *slot = *byte;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((data.len() as u64).to_be_bytes());
        v.extend(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn dff(audio: &[u8]) -> Vec<u8> {
        let mut chnl = 2u16.to_be_bytes().to_vec();
        chnl.extend(b"SLFTSRGT");
        let mut prop = b"SND ".to_vec();
        prop.extend(chunk(b"FS  ", &2_822_400u32.to_be_bytes()));
        prop.extend(chunk(b"CHNL", &chnl));
        let mut body = b"DSD ".to_vec();
        body.extend(chunk(b"PROP", &prop));
        body.extend(chunk(b"DSD ", audio));
        let mut file = b"FRM8".to_vec();
        file.extend((body.len() as u64).to_be_bytes());
        file.extend(body);
        file
    }

    #[test]
    fn deinterleaves_pads_and_reports_msb_first()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        // Left: 10..13, right: 20..23, interleaved byte by byte.
        let mut source = DffSource::new(Cursor::new(dff(&[10, 20, 11, 21, 12, 22, 13, 23])))?;
        assert!(source.info().msb_first);
        assert_eq!(source.info().sample_count, 32);

        let channels = source.read(-1, 6)?;
        let p = PAD_BYTE;
        assert_eq!(channels[0], [p, 10, 11, 12, 13, p]);
        assert_eq!(channels[1], [p, 20, 21, 22, 23, p]);
        Ok(())
    }
}
