//! Offline IP-to-region lookup using the ip2region `ip2region.xdb` database.
//!
//! Implements the xdb v2 memory-search algorithm (same as the official
//! `binding/c/xdb_searcher.c`):
//!   - 256-byte header
//!   - 256 x 256 vector index (each entry 8 bytes: s_ptr + e_ptr, little-endian)
//!   - segment index entries: start_ip(4) + end_ip(4) + data_len(2) + data_ptr(4)
//!   - region string format: `国家|区域|省份|城市|ISP`
//!
//! The database is loaded fully into memory once; lookups are pure in-memory
//! binary search (no disk I/O per query).

use std::sync::Arc;

const HEADER_LEN: usize = 256;
const VECTOR_INDEX_COLS: usize = 256;
const VECTOR_INDEX_SIZE: usize = 8; // s_ptr(4) + e_ptr(4)
const SEGMENT_INDEX_SIZE: usize = 14; // start_ip(4) + end_ip(4) + data_len(2) + data_ptr(4)

pub struct Ip2Region {
    data: Arc<Vec<u8>>,
}

impl Ip2Region {
    /// Load the xdb database from `path`. Returns None if the file is missing
    /// or invalid (lookups then gracefully return an empty region).
    pub fn load(path: &str) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        if bytes.len() <= HEADER_LEN {
            return None;
        }
        Some(Self {
            data: Arc::new(bytes),
        })
    }

    /// Look up an IPv4 address (dotted-quad string). Returns the region string
    /// `国家|区域|省份|城市|ISP`, or `None` if not found / not IPv4.
    pub fn search(&self, ip: &str) -> Option<String> {
        let ip_bytes = parse_ipv4(ip)?;
        let buf = &self.data;

        // Vector index: use the first two bytes of the IP.
        let il0 = ip_bytes[0] as usize;
        let il1 = ip_bytes[1] as usize;
        let idx = il0 * VECTOR_INDEX_COLS * VECTOR_INDEX_SIZE + il1 * VECTOR_INDEX_SIZE;
        let idx_off = HEADER_LEN + idx;
        if idx_off + VECTOR_INDEX_SIZE > buf.len() {
            return None;
        }
        let s_ptr = read_u32_le(buf, idx_off)? as usize;
        let e_ptr = read_u32_le(buf, idx_off + 4)? as usize;
        if s_ptr == 0 || e_ptr == 0 {
            return None;
        }

        // Binary search over segment index entries within [s_ptr, e_ptr).
        let mut l: usize = 0;
        let mut h: usize = (e_ptr - s_ptr) / SEGMENT_INDEX_SIZE;
        while l <= h {
            let m = (l + h) >> 1;
            let p = s_ptr + m * SEGMENT_INDEX_SIZE;
            if p + SEGMENT_INDEX_SIZE > buf.len() {
                return None;
            }
            let seg_start = &buf[p..p + 4];
            let seg_end = &buf[p + 4..p + 8];
            match ip_bytes.as_slice().cmp(seg_start) {
                std::cmp::Ordering::Less => {
                    if m == 0 {
                        break;
                    }
                    h = m - 1;
                }
                std::cmp::Ordering::Greater => {
                    if ip_bytes.as_slice() > seg_end {
                        l = m + 1;
                    } else {
                        // within [seg_start, seg_end]
                        let data_len = read_u16_le(buf, p + 8)? as usize;
                        let data_ptr = read_u32_le(buf, p + 10)? as usize;
                        return read_region(buf, data_ptr, data_len);
                    }
                }
                std::cmp::Ordering::Equal => {
                    let data_len = read_u16_le(buf, p + 8)? as usize;
                    let data_ptr = read_u32_le(buf, p + 10)? as usize;
                    return read_region(buf, data_ptr, data_len);
                }
            }
        }
        None
    }
}

fn read_region(buf: &[u8], ptr: usize, len: usize) -> Option<String> {
    let end = ptr.checked_add(len)?;
    if end > buf.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[ptr..end]).into_owned())
}

fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*buf.get(off)?, *buf.get(off + 1)?]))
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *buf.get(off)?,
        *buf.get(off + 1)?,
        *buf.get(off + 2)?,
        *buf.get(off + 3)?,
    ]))
}

/// Parse a dotted-quad IPv4 string into 4 bytes, or None if invalid.
fn parse_ipv4(ip: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse::<u8>().ok()?;
    }
    Some(out)
}
