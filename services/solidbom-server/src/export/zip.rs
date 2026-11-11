//! A minimal deterministic ZIP container writer (package SB-21).
//!
//! The Excel renderer packages an Office Open XML workbook as a ZIP archive. The entries are stored
//! uncompressed with a fixed timestamp so the same model always renders the same bytes, which is
//! what the export bundle's SHA-256 digest records. This module writes only what the workbook needs;
//! it is not a general archive tool.
//!
//! The archive is a plain stored ZIP: one local file header and its bytes per entry, one central
//! directory entry per file, and the end-of-central-directory record. Every local and central
//! record uses UTF-8 name flag bit 11 and the same fixed DOS date and time (1980-01-01T00:00:00, the
//! earliest DOS timestamp), so the bytes are a pure function of the entry names and contents. There
//! is no compression, no data descriptor, no comment and no extra field.
//!
//! Every size and offset is converted with checked arithmetic. Inputs that cannot be represented in
//! the ZIP32 format (a name or body of 4 GiB or more, or more than 65535 entries) yield an empty
//! archive instead of a panic; the real workbook is far below those limits.

/// The bit in the general-purpose flags that declares the entry name UTF-8.
const FLAG_UTF8: u16 = 0x0800;

/// The fixed modification time of every entry: midnight.
const DOS_TIME: u16 = 0;
/// The fixed modification date of every entry: 1980-01-01, the earliest DOS date.
const DOS_DATE: u16 = 0x0021;

/// The local file header signature, `PK\x03\x04`.
const LOCAL_HEADER: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
/// The central directory header signature, `PK\x01\x02`.
const CENTRAL_HEADER: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
/// The end-of-central-directory signature, `PK\x05\x06`.
const END_OF_CENTRAL_DIRECTORY: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];

/// The CRC-32 table for the reversed polynomial `0xEDB88320`, computed at compile time.
const CRC_TABLE: [u32; 256] = crc_table();

/// Build the CRC-32 lookup table once at compile time.
const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < table.len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            if value & 1 == 1 {
                value = 0xedb8_8320 ^ (value >> 1);
            } else {
                value >>= 1;
            }
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// The CRC-32 checksum of `bytes`, as the ZIP format stores it.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        let index = usize::from(((crc ^ u32::from(*byte)) & 0xff) as u8);
        crc = CRC_TABLE[index] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

/// Write one stored ZIP archive containing `entries`, in order.
///
/// The caller supplies `(name, bytes)` pairs; names are UTF-8 archive paths such as
/// `xl/worksheets/sheet1.xml`. The same entries always produce the same archive. An input that the
/// ZIP32 format cannot represent is answered with an empty vector (see the module docs).
pub(super) fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    try_archive(entries).unwrap_or_default()
}

/// Write the archive, or `None` when a name, body, offset or entry count does not fit ZIP32.
fn try_archive(entries: &[(&str, &[u8])]) -> Option<Vec<u8>> {
    let count = u16::try_from(entries.len()).ok()?;
    let mut local = Vec::new();
    let mut offsets = Vec::with_capacity(entries.len());
    for (name, data) in entries {
        offsets.push(u32::try_from(local.len()).ok()?);
        let name_len = u16::try_from(name.len()).ok()?;
        let size = u32::try_from(data.len()).ok()?;
        local.extend_from_slice(&LOCAL_HEADER);
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&DOS_TIME.to_le_bytes());
        local.extend_from_slice(&DOS_DATE.to_le_bytes());
        local.extend_from_slice(&crc32(data).to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&name_len.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        local.extend_from_slice(data);
    }

    let directory_offset = u32::try_from(local.len()).ok()?;
    let mut directory = Vec::new();
    for ((name, data), offset) in entries.iter().zip(offsets) {
        let name_len = u16::try_from(name.len()).ok()?;
        let size = u32::try_from(data.len()).ok()?;
        directory.extend_from_slice(&CENTRAL_HEADER);
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&DOS_TIME.to_le_bytes());
        directory.extend_from_slice(&DOS_DATE.to_le_bytes());
        directory.extend_from_slice(&crc32(data).to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&name_len.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_size = u32::try_from(directory.len()).ok()?;

    let mut archive = local;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&END_OF_CENTRAL_DIRECTORY);
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    Some(archive)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// One entry as the test reader returns it: the stored name, its bytes and the CRC the headers
    /// claim for it.
    struct ReadEntry {
        name: String,
        data: Vec<u8>,
        crc: u32,
    }

    /// A tiny independent reader for the stored archive this module writes.
    ///
    /// It reads the end record, then the central directory it points at, then each local file
    /// header and its stored bytes. It deliberately shares no code with the writer beyond the
    /// signature constants.
    fn read(name: &str, bytes: &[u8]) -> Vec<ReadEntry> {
        assert!(bytes.len() >= 22, "{name}: too short");
        let eocd = bytes.len() - 22;
        assert_eq!(bytes.get(eocd..eocd + 4), Some(&END_OF_CENTRAL_DIRECTORY[..]), "{name}: end record last");
        let count = usize::from(u16::from_le_bytes(bytes[eocd + 10..eocd + 12].try_into().unwrap()));
        let directory_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap());
        let directory_offset = usize::try_from(u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap())).unwrap();
        let mut position = directory_offset;
        let mut central = Vec::new();
        while bytes.get(position..position + 4) == Some(&CENTRAL_HEADER) {
            let flags = u16::from_le_bytes(bytes[position + 8..position + 10].try_into().unwrap());
            let method = u16::from_le_bytes(bytes[position + 10..position + 12].try_into().unwrap());
            let time = u16::from_le_bytes(bytes[position + 12..position + 14].try_into().unwrap());
            let date = u16::from_le_bytes(bytes[position + 14..position + 16].try_into().unwrap());
            let crc = u32::from_le_bytes(bytes[position + 16..position + 20].try_into().unwrap());
            let compressed = u32::from_le_bytes(bytes[position + 20..position + 24].try_into().unwrap());
            let uncompressed = u32::from_le_bytes(bytes[position + 24..position + 28].try_into().unwrap());
            let name_len = usize::from(u16::from_le_bytes(bytes[position + 28..position + 30].try_into().unwrap()));
            let extra_len = usize::from(u16::from_le_bytes(bytes[position + 30..position + 32].try_into().unwrap()));
            let comment_len = usize::from(u16::from_le_bytes(bytes[position + 32..position + 34].try_into().unwrap()));
            let offset = u32::from_le_bytes(bytes[position + 42..position + 46].try_into().unwrap());
            let entry_name = std::str::from_utf8(&bytes[position + 46..position + 46 + name_len]).unwrap().to_string();
            assert_eq!(flags & FLAG_UTF8, FLAG_UTF8, "{name}: UTF-8 name flag");
            assert_eq!(method, 0, "{name}: stored method");
            assert_eq!((time, date), (DOS_TIME, DOS_DATE), "{name}: fixed DOS timestamp");
            assert_eq!(compressed, uncompressed, "{name}: stored sizes agree");
            central.push((entry_name, crc, uncompressed, offset));
            position += 46 + name_len + extra_len + comment_len;
        }
        assert_eq!(position - directory_offset, usize::try_from(directory_size).unwrap(), "{name}: directory size");
        assert_eq!(central.len(), count, "{name}: entry count");

        let mut entries = Vec::new();
        for (entry_name, crc, size, offset) in central {
            let local = usize::try_from(offset).unwrap();
            assert_eq!(bytes.get(local..local + 4), Some(&LOCAL_HEADER[..]), "{name}: local header of {entry_name}");
            let local_name_len = usize::from(u16::from_le_bytes(bytes[local + 26..local + 28].try_into().unwrap()));
            let local_extra_len = usize::from(u16::from_le_bytes(bytes[local + 28..local + 30].try_into().unwrap()));
            let local_name = std::str::from_utf8(&bytes[local + 30..local + 30 + local_name_len]).unwrap();
            assert_eq!(local_name, entry_name, "{name}: local and central names agree");
            let data_start = local + 30 + local_name_len + local_extra_len;
            let data_end = data_start + usize::try_from(size).unwrap();
            let data = bytes[data_start..data_end].to_vec();
            let local_crc = u32::from_le_bytes(bytes[local + 14..local + 18].try_into().unwrap());
            assert_eq!(local_crc, crc, "{name}: local and central CRCs agree");
            assert_eq!(crc32(&data), crc, "{name}: stored CRC of {entry_name}");
            entries.push(ReadEntry { name: entry_name.to_string(), data, crc });
        }
        entries
    }

    #[test]
    fn entries_round_trip_with_their_names_bytes_and_crcs() {
        let first: Vec<u8> = (0..=255).collect();
        let archive = archive(&[
            ("[Content_Types].xml", b"<Types/>"),
            ("xl/worksheets/sheet1.xml", &first),
            ("nested/unicode-\u{00e4}\u{00f6}.bin", b"payload"),
        ]);
        let entries = read("archive", &archive);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "[Content_Types].xml");
        assert_eq!(entries[0].data, b"<Types/>");
        assert_eq!(entries[1].name, "xl/worksheets/sheet1.xml");
        assert_eq!(entries[1].data, first);
        assert_eq!(entries[2].name, "nested/unicode-\u{00e4}\u{00f6}.bin");
        for entry in &entries {
            assert_eq!(entry.crc, crc32(&entry.data), "{}", entry.name);
        }
        // The known CRC-32 of `123456789` pins the polynomial and the initialization.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn the_same_entries_always_produce_the_same_bytes() {
        let entries: [(&str, &[u8]); 2] = [("a.xml", b"<a/>"), ("b.xml", b"<b/>")];
        let first = archive(&entries);
        let second = archive(&entries);
        assert_eq!(first, second);
        assert!(!first.is_empty());
        // A fixed timestamp and no compressed body: the archive is stable across runs and hosts.
        assert_eq!(read("first", &first).len(), 2);
    }

    #[test]
    fn an_empty_archive_is_the_bare_end_record() {
        let archive = archive(&[]);
        assert_eq!(archive.len(), 22);
        assert_eq!(&archive[..4], &END_OF_CENTRAL_DIRECTORY);
        assert_eq!(&archive[10..12], &0u16.to_le_bytes());
        assert!(read("empty", &archive).is_empty());
    }

    #[test]
    fn large_stored_bodies_keep_their_sizes() {
        let body = vec![0x5au8; 300_000];
        let archive = archive(&[("big.bin", &body)]);
        let entries = read("big", &archive);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data.len(), body.len());
        assert_eq!(entries[0].crc, crc32(&body));
    }
}
