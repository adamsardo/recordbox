//! What rekordbox's file readers deliver, where recordbox's decoder (symphonia) differs.
//!
//! rekordbox 7.2.19 reads FLAC with libFLAC (`rbxfrm::FileReadFlac`, 0x1028144b8 …): the
//! timeline is STREAMINFO's total samples, every frame decoded, positions past the end read as
//! silence. symphonia 0.5.5 drops a FLAC file's last frame when it is shorter than the others
//! (its packet parser does not close the final fragment when bytes follow the last frame), so
//! `flac_tail` decodes that frame itself.
//!
//! MP3s are read with the mpg123 library the installed rekordbox ships (`vocal_rb_mp3`; traced in
//! `rbxfrm::FileReadMp3`: 16-bit output, nothing trimmed, rekordbox's frame table timeline), so
//! every analysis gets the same 16-bit samples rekordbox's does. PREPDECK_MP3=own keeps symphonia.
//!
//! AAC in MP4 is decoded with Apple's AudioToolbox the way rekordbox's `FileReadMp4` does it
//! (`apple_aac`); symphonia's AAC decoder is sample-aligned with it but differs by up to one
//! step of 16 bits on Apple-encoded files and by much more on some ffmpeg-encoded ones.

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::Decoder;
use symphonia::core::formats::Packet;

/// MP3s through rekordbox's mpg123 (installed and loadable, not switched off).
pub fn use_rb_mp3() -> bool {
    std::env::var("PREPDECK_MP3").map(|v| v != "own").unwrap_or(true) && crate::vocal_rb_mp3::library_loads()
}

/// AAC through AudioToolbox as rekordbox does (`apple_aac`): MP4 files (not raw ADTS `.aac`,
/// which rekordbox reads with a different reader, `FileReadAac`, not traced), unless
/// PREPDECK_AAC=own.
pub fn use_rb_aac(path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    cfg!(target_os = "macos") && ext != "aac" && std::env::var("PREPDECK_AAC").map(|v| v != "own").unwrap_or(true)
}

/// CRC-8 (polynomial 0x07) of a FLAC frame header.
fn crc8(b: &[u8]) -> u8 {
    let mut c = 0u8;
    for &x in b {
        c ^= x;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
    }
    c
}

/// Parses a FLAC frame header at `b[0..]`: (frame or sample number, block size in samples,
/// variable blocking) when the sync code, the coded fields and the header CRC-8 are valid.
pub fn frame_header(b: &[u8]) -> Option<(u64, u32, bool)> {
    if b.len() < 6 || b[0] != 0xFF || (b[1] & 0xFE) != 0xF8 {
        return None;
    }
    let variable = b[1] & 1 == 1;
    let bs_code = b[2] >> 4;
    let sr_code = b[2] & 0x0F;
    if bs_code == 0 || sr_code == 0x0F || (b[3] >> 4) > 10 || (b[3] >> 1) & 7 == 3 || (b[3] >> 1) & 7 == 7 || b[3] & 1 != 0 {
        return None;
    }
    // UTF-8-style coded number.
    let mut p = 4usize;
    let first = b[p];
    let (mut num, extra) = match first {
        0x00..=0x7F => (first as u64, 0),
        0xC0..=0xDF => ((first & 0x1F) as u64, 1),
        0xE0..=0xEF => ((first & 0x0F) as u64, 2),
        0xF0..=0xF7 => ((first & 0x07) as u64, 3),
        0xF8..=0xFB => ((first & 0x03) as u64, 4),
        0xFC..=0xFD => ((first & 0x01) as u64, 5),
        0xFE => (0, 6),
        _ => return None,
    };
    p += 1;
    for _ in 0..extra {
        let c = *b.get(p)?;
        if c & 0xC0 != 0x80 {
            return None;
        }
        num = (num << 6) | (c & 0x3F) as u64;
        p += 1;
    }
    let block = match bs_code {
        1 => 192,
        2..=5 => 576u32 << (bs_code - 2),
        6 => {
            p += 1;
            *b.get(p - 1)? as u32 + 1
        }
        7 => {
            p += 2;
            u16::from_be_bytes([*b.get(p - 2)?, *b.get(p - 1)?]) as u32 + 1
        }
        _ => 256u32 << (bs_code - 8),
    };
    p += match sr_code {
        12 => 1,
        13 | 14 => 2,
        _ => 0,
    };
    let crc = *b.get(p)?;
    (crc8(&b[..p]) == crc).then_some((num, block, variable))
}

/// Decodes the FLAC frames symphonia's demuxer did not deliver: `have` frames were decoded of
/// `total` (STREAMINFO). Finds the frame header that starts at sample `have` (searched from the
/// end of the file, header CRC checked) and decodes everything from there to the end of the
/// file with the track's decoder. Returns interleaved samples as symphonia gives them (all
/// channels), at most `total - have` frames, or None when no such frame is found.
pub fn flac_tail(path: &str, decoder: &mut dyn Decoder, track_id: u32, have: u64, total: u64, fixed_block: Option<u32>) -> Option<(Vec<f32>, usize)> {
    if have >= total {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    // The missing frames are at most a few hundred kB from the end.
    let lo = bytes.len().saturating_sub(4 << 20);
    let mut start = None;
    let mut i = bytes.len().saturating_sub(6);
    while i > lo {
        i -= 1;
        if bytes[i] != 0xFF || (bytes[i + 1] & 0xFE) != 0xF8 {
            continue;
        }
        if let Some((num, _, variable)) = frame_header(&bytes[i..]) {
            let first_sample = if variable { num } else { num * fixed_block.unwrap_or(0) as u64 };
            if first_sample == have {
                start = Some(i);
                break;
            }
            if first_sample < have {
                break;
            }
        }
    }
    let Some(start) = start else {
        eprintln!("flac tail: no frame header for sample {have} near the end of {path}");
        return None;
    };
    let mut out = vec![];
    let mut ch = 0usize;
    // Frames one after another: each packet runs to the next valid header that continues the
    // count (or the end of the file).
    let mut pos = start;
    let mut sample = have;
    while pos < bytes.len() && sample < total {
        let (num, block, variable) = frame_header(&bytes[pos..])?;
        let first = if variable { num } else { num * fixed_block.unwrap_or(block) as u64 };
        if first != sample {
            break;
        }
        let mut end = bytes.len();
        let mut j = pos + 2;
        while j + 6 <= bytes.len() {
            if bytes[j] == 0xFF && (bytes[j + 1] & 0xFE) == 0xF8 {
                if let Some((n2, _, v2)) = frame_header(&bytes[j..]) {
                    let f2 = if v2 { n2 } else { n2 * fixed_block.unwrap_or(block) as u64 };
                    if f2 == sample + block as u64 {
                        end = j;
                        break;
                    }
                }
            }
            j += 1;
        }
        let pkt = Packet::new_from_slice(track_id, sample, block as u64, &bytes[pos..end]);
        let buf = match decoder.decode(&pkt) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("flac tail: frame at sample {sample} did not decode: {e}");
                return None;
            }
        };
        let spec = *buf.spec();
        ch = spec.channels.count();
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        let frames = (sb.samples().len() / ch.max(1)) as u64;
        let keep = frames.min(total - sample) as usize;
        out.extend_from_slice(&sb.samples()[..keep * ch]);
        sample += frames;
        pos = end;
    }
    (!out.is_empty()).then_some((out, ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frame_headers() {
        // Fixed blocking, block size code 0xC (4096), 44.1 kHz (code 9), stereo independent,
        // 16 bit, frame number 5.
        let mut h = vec![0xFF, 0xF8, 0xC9, 0x08, 0x05];
        h.push(crc8(&h));
        assert_eq!(frame_header(&h), Some((5, 4096, false)));
        h[5] ^= 1;
        assert_eq!(frame_header(&h), None);
        // Block size stored in 16 bits (code 7): 3154 samples, two-byte frame number 1470.
        let mut h = vec![0xFF, 0xF8, 0x79, 0x08, 0xD6, 0xBE, 0x0C, 0x51];
        h.push(crc8(&h));
        assert_eq!(frame_header(&h), Some((1470, 3154, false)));
    }
}

/// AAC in MP4 (M4A) as rekordbox reads it (`rbxfrm::FileReadMp4`, 0x102820700 … 0x102821154):
/// Apple's AudioToolbox decodes it (`AudioFileOpenURL`, `AudioConverterNew` from the file's data
/// format to 16-bit signed packed integer PCM at the file's rate and channel count, the file's
/// magic cookie, and, when the file has packet table info, the converter's prime info set to
/// zero leading and trailing frames), fed one packet at a time with `AudioFileReadPacketData`.
/// The timeline is the packets' own (sample table durations from 0): the encoder's priming
/// samples stay, nothing is trimmed. rekordbox's reader then turns the 16-bit values into floats
/// (/ 32768) and back; returned here as 16-bit interleaved stereo and the sample rate.
#[cfg(target_os = "macos")]
pub mod apple_aac {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default, Debug)]
    pub struct Asbd {
        pub sample_rate: f64,
        pub format_id: u32,
        pub format_flags: u32,
        pub bytes_per_packet: u32,
        pub frames_per_packet: u32,
        pub bytes_per_frame: u32,
        pub channels_per_frame: u32,
        pub bits_per_channel: u32,
        pub reserved: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct PacketDesc {
        start_offset: i64,
        variable_frames: u32,
        data_byte_size: u32,
    }

    #[repr(C)]
    struct AudioBuffer {
        channels: u32,
        size: u32,
        data: *mut c_void,
    }

    #[repr(C)]
    struct BufferList {
        n: u32,
        buf: AudioBuffer,
    }

    #[repr(C)]
    #[derive(Default)]
    struct PrimeInfo {
        leading: u32,
        trailing: u32,
    }

    type OSStatus = i32;
    type FileId = *mut c_void;
    type Converter = *mut c_void;
    type InputProc = extern "C" fn(Converter, *mut u32, *mut BufferList, *mut *mut PacketDesc, *mut c_void) -> OSStatus;

    #[link(name = "AudioToolbox", kind = "framework")]
    extern "C" {
        fn AudioFileOpenURL(url: *const c_void, perm: i8, hint: u32, out: *mut FileId) -> OSStatus;
        fn AudioFileClose(f: FileId) -> OSStatus;
        fn AudioFileGetProperty(f: FileId, id: u32, size: *mut u32, data: *mut c_void) -> OSStatus;
        fn AudioFileGetPropertyInfo(f: FileId, id: u32, size: *mut u32, writable: *mut u32) -> OSStatus;
        fn AudioFileReadPacketData(f: FileId, cache: u8, bytes: *mut u32, descs: *mut PacketDesc, start: i64, packets: *mut u32, out: *mut c_void) -> OSStatus;
        fn AudioConverterNew(src: *const Asbd, dst: *const Asbd, out: *mut Converter) -> OSStatus;
        fn AudioConverterDispose(c: Converter) -> OSStatus;
        fn AudioConverterSetProperty(c: Converter, id: u32, size: u32, data: *const c_void) -> OSStatus;
        fn AudioConverterFillComplexBuffer(c: Converter, input: InputProc, user: *mut c_void, packets: *mut u32, out: *mut BufferList, descs: *mut PacketDesc) -> OSStatus;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(alloc: *const c_void, path: *const u8, len: isize, dir: u8) -> *const c_void;
        fn CFRelease(p: *const c_void);
    }

    const fn fourcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    struct Feed {
        file: FileId,
        next: i64,
        total: i64,
        max_packet: u32,
        data: Vec<u8>,
        desc: PacketDesc,
    }

    extern "C" fn feed(_c: Converter, packets: *mut u32, out: *mut BufferList, descs: *mut *mut PacketDesc, user: *mut c_void) -> OSStatus {
        unsafe {
            let f = &mut *(user as *mut Feed);
            if f.next >= f.total {
                *packets = 0;
                (*out).buf.size = 0;
                (*out).buf.data = std::ptr::null_mut();
                return 0;
            }
            // One packet per call (rekordbox reads at most 0x800 / (bytes per frame) packets;
            // the converter asks for one at a time for AAC).
            let mut n = 1u32;
            let mut bytes = f.max_packet.max(1);
            if f.data.len() < bytes as usize {
                f.data.resize(bytes as usize, 0);
            }
            let st = AudioFileReadPacketData(f.file, 1, &mut bytes, &mut f.desc, f.next, &mut n, f.data.as_mut_ptr() as *mut c_void);
            if st != 0 || n == 0 {
                *packets = 0;
                (*out).buf.size = 0;
                (*out).buf.data = std::ptr::null_mut();
                return st;
            }
            f.next += n as i64;
            *packets = n;
            (*out).buf.size = bytes;
            (*out).buf.data = f.data.as_mut_ptr() as *mut c_void;
            if !descs.is_null() {
                f.desc.start_offset = 0;
                *descs = &mut f.desc;
            }
            0
        }
    }

    /// (16-bit interleaved stereo, sample rate). None when the file is not AAC or AudioToolbox
    /// refuses it.
    pub fn decode(path: &str) -> Option<(Vec<i16>, u32)> {
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), path.as_ptr(), path.len() as isize, 0);
            if url.is_null() {
                return None;
            }
            let mut file: FileId = std::ptr::null_mut();
            let st = AudioFileOpenURL(url, 1, 0, &mut file);
            CFRelease(url);
            if st != 0 || file.is_null() {
                return None;
            }
            struct Close(FileId);
            impl Drop for Close {
                fn drop(&mut self) {
                    unsafe {
                        AudioFileClose(self.0);
                    }
                }
            }
            let _close = Close(file);
            let mut src = Asbd::default();
            let mut sz = std::mem::size_of::<Asbd>() as u32;
            if AudioFileGetProperty(file, fourcc(b"dfmt"), &mut sz, &mut src as *mut _ as *mut c_void) != 0 {
                return None;
            }
            let aac = [fourcc(b"aac "), fourcc(b"aach"), fourcc(b"aacl"), fourcc(b"aacp"), fourcc(b"aacs")];
            if !aac.contains(&src.format_id) || !(1..=2).contains(&src.channels_per_frame) {
                return None;
            }
            let ch = src.channels_per_frame;
            let dst = Asbd {
                sample_rate: src.sample_rate,
                format_id: fourcc(b"lpcm"),
                format_flags: 0xC,
                bytes_per_packet: 2 * ch,
                frames_per_packet: 1,
                bytes_per_frame: 2 * ch,
                channels_per_frame: ch,
                bits_per_channel: 16,
                reserved: 0,
            };
            let mut conv: Converter = std::ptr::null_mut();
            if AudioConverterNew(&src, &dst, &mut conv) != 0 || conv.is_null() {
                return None;
            }
            struct Dispose(Converter);
            impl Drop for Dispose {
                fn drop(&mut self) {
                    unsafe {
                        AudioConverterDispose(self.0);
                    }
                }
            }
            let _dispose = Dispose(conv);
            // Magic cookie ('mgic') into the converter ('dmgc').
            let (mut csz, mut w) = (0u32, 0u32);
            if AudioFileGetPropertyInfo(file, fourcc(b"mgic"), &mut csz, &mut w) == 0 && csz > 0 {
                let mut cookie = vec![0u8; csz as usize];
                if AudioFileGetProperty(file, fourcc(b"mgic"), &mut csz, cookie.as_mut_ptr() as *mut c_void) != 0 {
                    return None;
                }
                if AudioConverterSetProperty(conv, fourcc(b"dmgc"), csz, cookie.as_ptr() as *const c_void) != 0 {
                    return None;
                }
            }
            // Packet table info readable ('pnfo'): prime info ('prim') 0 / 0, so no priming
            // or remainder frames are dropped.
            let mut pnfo = [0u8; 16];
            let mut psz = 16u32;
            if AudioFileGetProperty(file, fourcc(b"pnfo"), &mut psz, pnfo.as_mut_ptr() as *mut c_void) == 0 {
                let p = PrimeInfo::default();
                if AudioConverterSetProperty(conv, fourcc(b"prim"), 8, &p as *const _ as *const c_void) != 0 {
                    return None;
                }
            }
            let mut total: u64 = 0;
            let mut tsz = 8u32;
            if AudioFileGetProperty(file, fourcc(b"pcnt"), &mut tsz, &mut total as *mut _ as *mut c_void) != 0 {
                return None;
            }
            let mut maxp: u32 = 0;
            let mut msz = 4u32;
            if AudioFileGetProperty(file, fourcc(b"psze"), &mut msz, &mut maxp as *mut _ as *mut c_void) != 0 {
                maxp = 1 << 16;
            }
            let mut f = Feed { file, next: 0, total: total as i64, max_packet: maxp, data: vec![], desc: PacketDesc::default() };
            let fpp = src.frames_per_packet.max(1);
            let mut out: Vec<i16> = Vec::with_capacity((total as usize) * fpp as usize * 2);
            let mut buf = vec![0i16; (fpp * ch) as usize];
            loop {
                let mut frames = fpp;
                let mut bl = BufferList { n: 1, buf: AudioBuffer { channels: ch, size: frames * 2 * ch, data: buf.as_mut_ptr() as *mut c_void } };
                let st = AudioConverterFillComplexBuffer(conv, feed, &mut f as *mut Feed as *mut c_void, &mut frames, &mut bl, std::ptr::null_mut());
                if frames == 0 {
                    break;
                }
                let got = &buf[..(frames * ch) as usize];
                if ch == 1 {
                    for v in got {
                        out.push(*v);
                        out.push(*v);
                    }
                } else {
                    out.extend_from_slice(got);
                }
                if st != 0 {
                    break;
                }
            }
            (!out.is_empty()).then_some((out, src.sample_rate as u32))
        }
    }
}

/// rekordbox's sample rate converter (`rbxfrm::SampleRateConverter` with
/// `f2dsp::SamplingFunctionSRC<Float2>` / `c_src<Float2>`, 0x10284b490, 0x10288b0ec …
/// 0x10288d91c), which its reader puts in front of the analysis for files that are neither
/// 44.1 nor 48 kHz (`CBeatAnalyzeThread::analyze`: a multiple of 8000 Hz goes to 48 kHz,
/// anything else to 44.1 kHz; `ReaderImpl::setReader` adds the converter when the rates differ).
///
/// The converter is a polyphase FIR with 29 taps per phase: L = out/50 phases, M = in/50 input
/// steps; output k (phase p = k mod L) is Σ_j h_p[j] · x[⌊kM/L⌋ − 28 + j] (inputs before the
/// start are 0), accumulated in f32 with fused multiply-adds in the order j = 0, 28, 1, 27 … 13,
/// 15, 14 (`convolute_fine`). The taps come from a Kaiser-windowed sinc of 29·L points designed
/// in double precision (`initIdxAndIrForKaiser`): cut-off fc / max(L, M) of the virtual rate,
/// fc and β from a per-rate table (`SamplingFunctionSRC::init`), the window's Bessel function
/// as a 22-term series, h_p[j] = (float)(L · ir[L·(j + 1 + ⌊Mp/L⌋) − M·p − 1]). The output has
/// as many frames as outputs whose newest input lies inside the track (`calcOutputNum`); the
/// reader feeds zeros past the end. The 28-sample history makes the output lag the input by the
/// filter's half length (14 input samples), as in rekordbox.
pub mod rb_src {
    pub const TAPS: usize = 29;

    pub struct Src {
        pub l: usize,
        pub m: usize,
        pub idx: Vec<usize>,
        pub taps: Vec<[f32; TAPS]>,
    }

    /// (fc, β) of `SamplingFunctionSRC<Float2>::init` for a conversion.
    pub fn params(inr: u32, outr: u32) -> (f64, f64) {
        let c = f64::from_bits;
        match (inr, outr) {
            (44100, 48000) => (c(0x3FE147AE147AE148), 8.0),
            (44100, 96000) | (44100, 88200) => (c(0x3FE28F5C28F5C28F), 10.0),
            (48000, 44100) => (c(0x3FE0A3D70A3D70A4), 8.0),
            (48000, 96000) => (c(0x3FDCCCCCCCCCCCCD), 10.0),
            (96000, 44100) | (88200, 44100) => (c(0x3FE199999999999A), 6.0),
            (96000, 48000) => (c(0x3FDE147AE147AE14), 10.0),
            (44100, 176400) => (c(0x3FDEB851EB851EB8), 9.0),
            (44100, 192000) => (0.48, 9.0),
            _ => (c(0x3FDCCCCCCCCCCCCD), 12.0),
        }
    }

    /// The converter for `inr` → `outr` Hz; None where rekordbox's refuses (rates outside
    /// 8000 … 200,000 Hz or not multiples of 50).
    pub fn new(inr: u32, outr: u32) -> Option<Src> {
        if !(8000..=200_000).contains(&inr) || !(8000..=200_000).contains(&outr) || inr % 50 != 0 || outr % 50 != 0 {
            return None;
        }
        let (fc, beta) = params(inr, outr);
        let l = (outr / 50) as usize;
        let m = (inr / 50) as usize;
        let fcn = fc / l.max(m) as f64;
        let n = TAPS * l;
        let half = -((if (n as i32) < 0 { n as i32 + 1 } else { n as i32 }) >> 1);
        let two_pi_fc = fcn * f64::from_bits(0x401921fb54442d18);
        let pi = f64::from_bits(0x400921fb54442d18);
        let step = {
            let r = 1.0 / (n as f64 - 1.0);
            r + r
        };
        let hb = beta * 0.5;
        // 1 / (k!)², k = 0 … 21 (exact factorials).
        let inv_f2: Vec<f64> = (0..22)
            .map(|k: u32| {
                let mut f = 1.0f64;
                for i in 2..=k {
                    f *= i as f64;
                }
                1.0 / (f * f)
            })
            .collect();
        let mut ir = vec![0f64; n];
        for (i, v) in ir.iter_mut().enumerate() {
            let t = half + i as i32;
            let sinc = if t == 0 { fcn + fcn } else { (two_pi_fc * t as f64).sin() / (t as f64 * pi) };
            let u = i as f64 * step;
            let um1 = u + -1.0;
            let sq = u.mul_add(um1, -um1);
            let x = (1.0 - sq).sqrt() * hb;
            let (mut ix, mut ib) = (0f64, 0f64);
            let mut e = 0f64;
            for k in 0..22 {
                let p = e + e;
                ix = inv_f2[k].mul_add(x.powf(p), ix);
                ib = inv_f2[k].mul_add(hb.powf(p), ib);
                e += 1.0;
            }
            *v = ix * sinc / ib;
        }
        let idx: Vec<usize> = (0..l).map(|p| m * p / l).collect();
        let lf = l as f64;
        let taps = (0..l)
            .map(|p| {
                let mut h = [0f32; TAPS];
                for (j, t) in h.iter_mut().enumerate() {
                    let k = l + l * (j + idx[p]) - m * p - 1;
                    *t = (ir[k] * lf * 1.0) as f32;
                }
                h
            })
            .collect();
        Some(Src { l, m, idx, taps })
    }

    impl Src {
        /// Frames out for `n_in` frames in (`calcOutputNum` on a fresh converter): every k with
        /// ⌊kM/L⌋ < n_in.
        pub fn out_len(&self, n_in: usize) -> usize {
            (n_in * self.l).div_ceil(self.m)
        }

        /// Converts interleaved stereo (as the reader's floats) from the start of a track.
        pub fn run(&self, x: &[f32]) -> Vec<f32> {
            let n_in = x.len() / 2;
            let n_out = self.out_len(n_in);
            let mut out = vec![0f32; 2 * n_out];
            let at = |n: i64, c: usize| -> f32 {
                if n < 0 || n as usize >= n_in {
                    0.0
                } else {
                    x[2 * n as usize + c]
                }
            };
            for k in 0..n_out {
                let p = k % self.l;
                let base = ((k / self.l) * self.m + self.idx[p]) as i64 - 28;
                let h = &self.taps[p];
                for c in 0..2 {
                    let mut acc = 0f32;
                    for j in 0..14 {
                        acc = h[j].mul_add(at(base + j as i64, c), acc);
                        acc = h[28 - j].mul_add(at(base + 28 - j as i64, c), acc);
                    }
                    acc = h[14].mul_add(at(base + 14, c), acc);
                    out[2 * k + c] = acc;
                }
            }
            out
        }
    }
}

/// The rate rekordbox's analysis reads a file at (`CBeatAnalyzeThread::analyze`): 44.1 and 48 kHz
/// as they are, multiples of 8000 Hz at 48 kHz, anything else at 44.1 kHz.
pub fn analysis_rate(sr: u32) -> u32 {
    if sr == 44100 || sr == 48000 {
        sr
    } else if sr % 8000 == 0 {
        48000
    } else {
        44100
    }
}

/// The audio rekordbox's analysis gets for a decoded file at another rate than 44.1 / 48 kHz:
/// converted to the analysis rate with rekordbox's converter (None when no conversion is due,
/// or with PREPDECK_RBSRC=own).
pub fn for_analysis(dec: &crate::audio::Decoded) -> Option<crate::audio::Decoded> {
    let to = analysis_rate(dec.sr);
    if to == dec.sr || std::env::var("PREPDECK_RBSRC").map(|v| v == "own").unwrap_or(false) {
        return None;
    }
    let src = rb_src::new(dec.sr, to)?;
    let samples = src.run(&dec.samples);
    let frames = samples.len() / 2;
    Some(crate::audio::Decoded { sr: to, samples, raw_frames: frames, damaged: dec.damaged, truncated: dec.truncated })
}

#[cfg(test)]
mod src_tests {
    /// The port against rekordbox's own converter run by the oracle harness `rbsrc.c` (calls
    /// `SamplingFunctionSRC<Float2>` in the 7.2.19 slice the way `SampleRateConverter::fileRead`
    /// drives it). PREPDECK_SRC_IN = f32 stereo input, PREPDECK_SRC_OUT = '|'-separated
    /// "in_rate:out_rate:oracle_output.f32". Run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn src_against_oracle() {
        let (Ok(inp), Ok(outs)) = (std::env::var("PREPDECK_SRC_IN"), std::env::var("PREPDECK_SRC_OUT")) else { return };
        let rd = |p: &str| -> Vec<f32> { std::fs::read(p).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect() };
        let x = rd(&inp);
        for o in outs.split('|') {
            let mut it = o.splitn(3, ':');
            let (a, b, f) = (it.next().unwrap().parse::<u32>().unwrap(), it.next().unwrap().parse::<u32>().unwrap(), it.next().unwrap());
            let want = rd(f);
            let got = super::rb_src::new(a, b).unwrap().run(&x);
            let same = got.iter().zip(&want).filter(|(p, q)| p.to_bits() == q.to_bits()).count();
            let maxd = got.iter().zip(&want).map(|(p, q)| (p - q).abs()).fold(0f32, f32::max);
            eprintln!("SRC {a} -> {b}: frames {} (oracle {}), bit-identical {same}/{}, max difference {maxd:e}", got.len() / 2, want.len() / 2, want.len());
        }
    }
}

#[cfg(test)]
mod src_beat {
    /// For each PREPDECK_AUDIO file ('|'-separated): the standard beat analysis of what this
    /// application's reader path gives (rekordbox's converter port to the analysis rate, one
    /// frame less), as "SRCBEAT <rate> <bpm×100> <first beat sample> <first beat number> <beats>"
    /// for comparison with the original converter + original beat analyser (scratch script).
    #[test]
    #[ignore]
    fn src_beat_check() {
        let Ok(list) = std::env::var("PREPDECK_AUDIO") else { return };
        for p in list.split('|') {
            let d = crate::audio::decode_file(p).unwrap();
            let conv = super::for_analysis(&d);
            let a = conv.as_ref().unwrap_or(&d);
            let (bpm, grid) = crate::analysis::rb_beat1::tempo_and_grid(&a.samples, a.sr, false, 70.0, 180.0).unwrap();
            let g = grid.unwrap();
            let first = g.anchors[0];
            eprintln!("SRCBEAT {} {} {:.3} {} frames {} :: {p}", a.sr, (bpm * 100.0).round(), first.time * 1000.0, first.beat, a.samples.len() / 2);
        }
    }
}
