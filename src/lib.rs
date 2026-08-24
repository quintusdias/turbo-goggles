extern crate byteorder;

use std::fs::File;
use std::io::{self, BufReader, Cursor, ErrorKind, Read, Seek, SeekFrom};
use std::collections::HashMap;
use std::fmt;

use byteorder::{ByteOrder, ReadBytesExt, BE, LE};


#[derive(Debug)]
pub enum TagPayload {
    U8val(Vec<u8>),
    U16val(Vec<u16>),
    U32val(Vec<u32>),
    I8val(Vec<i8>),
    I16val(Vec<i16>),
    I32val(Vec<i32>),
    F64val(Vec<f64>),
    Sval(String),
}

#[derive(Debug)]
pub enum TiffError {
    Io(std::io::Error),
    Endian(String),
    Class(String),
    Datatype(String),
    TagValue(String),
}

// Implement Display to control the user-facing message.
impl fmt::Display for TiffError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TiffError::Io(e) => write!(f, "IO error: {}", e),
            TiffError::Endian(msg) => write!(f, "Endian mismatch: {}", msg),
            TiffError::Class(msg) => write!(f, "TIFF Class: {}", msg),
            TiffError::Datatype(msg) => write!(f, "Tag Datatype: {}", msg),
            TiffError::TagValue(msg) => write!(f, "Tag Value: {}", msg),
        }
    }
}

impl From<std::io::Error> for TiffError {
    fn from(err: std::io::Error) -> Self {
        TiffError::Io(err)
    }
}

/// 
#[derive(Debug)]
pub struct Tiff {
    pub path: String,
    pub little_endian: bool,
    pub ifd: HashMap<u16, TagPayload>,
}

impl Tiff {

    pub fn new(path: &str) -> Result<Tiff, TiffError> {

        let little_endian = Tiff::determine_endianness(path)?;
        let ifd = Tiff::parse(path, little_endian)?;

        Ok(
            Tiff {
                path: String::from(path),
                little_endian: little_endian,
                ifd: ifd,
            }
        )
    }

    pub fn determine_endianness(path: &str) -> Result<bool, TiffError> {
	// Determine if the TIFF is little-endian (true) or big-endian
	// (false).

        let file = File::open(path)?;
        let mut buf_reader = BufReader::new(file);

        // read the tiff header
        let mut start = [0u8; 2];
        buf_reader.read_exact(&mut start)?;

        match &start {
            b"II" => Ok(true),
            b"MM" => Ok(false),
            _ => Err(TiffError::Endian(format!("Failed to parse endianness from {:?}", start))),
        }

    }

    pub fn parse(path: &str, little_endian: bool) -> Result<HashMap<u16, TagPayload>, TiffError> {

	// Parse the TIFF file, return 1st IFD.

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        // Seek past endianness marker.
        reader.seek(SeekFrom::Start(2 as u64))?;

        match little_endian {
            true => Tiff::read_1st_ifd::<LE, _>(&mut reader),
            false => Tiff::read_1st_ifd::<BE, _>(&mut reader),
        }

    }

    fn read_1st_ifd<E, R>(reader: &mut R) -> Result<HashMap<u16, TagPayload>, TiffError>
    where
        E: ByteOrder,
        R: ReadBytesExt + Seek,
    {
        let mut dtype2size: HashMap<u16, usize> = HashMap::new();

        // TIFF_ASCII is 1 bytes.
        dtype2size.insert(2 as u16, 1 as usize);

        // TIFF_SHORT is 2 bytes.
        dtype2size.insert(3 as u16, 2 as usize);

        // TIFF_LONG is 4 bytes.
        dtype2size.insert(4 as u16, 4 as usize);

        // TIFF_RATIONAL is 8 bytes.
        dtype2size.insert(5 as u16, 8 as usize);

        // read the rest of the header
        let mut header = [0u8; 6];
        reader.read_exact(&mut header)?;
    
        // verify the TIFF version
        if ! (header[..2] == [0u8, 42u8] || header[..2] == [42u8, 0]) {
            return Err(TiffError::Class("TIFF version was not verified.".to_string()));
        }
    
        // Get the offset to the first IFD
        let offset = header[2..].as_ref().read_u32::<E>()?;
    
        // seek to that first IFD
        reader.seek(SeekFrom::Start(offset as u64))?;
    
        // Read the number of tag entries
        let mut buffer = [0u8; 2];
        reader.read_exact(&mut buffer)?;
        let num_tags = buffer.as_ref().read_u16::<E>()?;
    
        // read in the first IFD
        let tag_buf_size = num_tags * 12;
        let mut ifd_buffer = vec![0u8; tag_buf_size as usize];
        reader.read_exact(&mut ifd_buffer)?;
    
        let mut offset: usize;
    
        let mut payload_size: usize;
    
        let mut tags: HashMap<u16, TagPayload> = HashMap::new();
    
        for idx in 0..num_tags {
    
            offset = idx as usize * 12;
    
            // Read the ID
            let tag_id = ifd_buffer[offset..(offset + 2)].as_ref().read_u16::<E>()?;
            let tag_dtype = ifd_buffer[(offset + 2)..(offset + 4)].as_ref().read_u16::<E>()?;
            let tag_count = ifd_buffer[(offset + 4)..(offset + 8)].as_ref().read_u32::<E>()?;
    
            // if the size of they payload exceeds 4 bytes, then we have
            // to consider the "payload" to be the offset to the real
            // payload.
            match dtype2size.get(&tag_dtype) {
                Some(item_size) => {
                    payload_size = *item_size * tag_count as usize;
    
                },
                None => {
                    let message = format!("Unhandled dtype {}", tag_dtype);
                    return Err(TiffError::Datatype(message));
                }
            }
    
            let mut payload_buffer = vec![0u8; payload_size];
    
            if payload_size > 4 {
    
                // The space pointed to by the offset field holds the value
    
                // read the location of the payload, seek to it,
                // read the payload, then go back to where we were.
                let offset = ifd_buffer[offset + 8..offset + 12].as_ref().read_u32::<E>()?;
                let old_pos = reader.seek(SeekFrom::Start(offset as u64))?;
                reader.read_exact(&mut payload_buffer)?;
                reader.seek(SeekFrom::Start(old_pos))?;
    
            } else {
    
                // The space at the end of the tag holds the value.
                payload_buffer = ifd_buffer[offset + 8..offset + 12]
                                    .iter()
                                    .map(|&s| s)
                                    .collect();
    
            }
    
            let payload = Tiff::tag_buffer_to_value::<E>(tag_dtype, tag_count, &payload_buffer)?;
            tags.insert(tag_id, payload);
    
        }
    
        Ok(tags)
    }

    fn tag_buffer_to_value<E>(tag_dtype: u16, count: u32, payload: &[u8]) -> Result<TagPayload, TiffError> 
    where
        E: ByteOrder,
    {
    
        match tag_dtype {
    
            // ASCII
            2 => {
                let v = str::from_utf8(payload)
                    .map_err(|e: std::str::Utf8Error| {
                        TiffError::TagValue(format!("Utf8Error: {e}"))
                    })?;
                let v2 = v.trim_matches('\0');
                Ok(TagPayload::Sval(String::from(v2)))
            },
    
            // SHORT
            3 => {
                let mut v = vec![0u16; count as usize];
                let mut cursor = Cursor::new(payload);
                cursor.read_u16_into::<E>(&mut v)?;
                Ok(TagPayload::U16val(v))
            },
    
            // LONG
            4 => {
                let mut v = vec![0u32; count as usize];
                let mut cursor = Cursor::new(payload);
                cursor.read_u32_into::<E>(&mut v)?;
                Ok(TagPayload::U32val(v))
            },
    
            // RATIONAL
            5 => {
                let mut v = vec![0u32; (count * 2) as usize];
                let mut cursor = Cursor::new(payload);
                cursor.read_u32_into::<E>(&mut v)?;
    
                let mut vr = vec![0f64; count as usize];
                for idx in 0..count as usize {
                    vr[idx] = (v[idx * 2] as f64 / v[idx * 2 + 1] as f64) as f64;
                }
    
                Ok(TagPayload::F64val(vr))
            },
    
            _ => {
                let message = format!("Unhandled dtype {}", tag_dtype);
                return Err(TiffError::Datatype(message));
            }
    
        }
    
    }
}




#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_endian_tiff() {
        // Scenario:  read a big-endian TIFF
        //
        // Expected Result:  tag values are verified
        let t = Tiff::new("test-data/asf-logo.tif").unwrap();

        let values = match t.ifd.get(&256u16) {
            Some(TagPayload::U32val(v)) => v,
            _ => panic!("not a u32"),
        };
        assert_eq!(*values, vec![169u32; 1]);

        let values = match t.ifd.get(&257u16) {
            Some(TagPayload::U32val(v)) => v,
            _ => panic!("not a u32"),
        };
        assert_eq!(*values, vec![51u32; 1]);

        let values = match t.ifd.get(&258u16) {
            Some(TagPayload::U16val(v)) => v,
            _ => panic!("not a u16"),
        };
        assert_eq!(*values, vec![8u16; 4]);

        let values = match t.ifd.get(&259u16) {
            Some(TagPayload::U16val(v)) => v,
            _ => panic!("not a u16"),
        };
        assert_eq!(*values, vec![1u16; 1]);

        let values = match t.ifd.get(&262u16) {
            Some(TagPayload::U16val(v)) => v,
            _ => panic!("not a u16"),
        };
        assert_eq!(*values, vec![2u16; 1]);

        let values = match t.ifd.get(&273u16) {
            Some(TagPayload::U32val(v)) => v,
            _ => panic!("not a u32"),
        };
        assert_eq!(values[0], 250);

        let values = match t.ifd.get(&277u16) {
            Some(TagPayload::U16val(v)) => v,
            _ => panic!("not a u16"),
        };
        assert_eq!(*values, vec![4u16; 1]);

        let values = match t.ifd.get(&278u16) {
            Some(TagPayload::U32val(v)) => v,
            _ => panic!("not a u32"),
        };
        assert_eq!(*values, vec![8u32; 1]);

        let values = match t.ifd.get(&279u16) {
            Some(TagPayload::U32val(v)) => v,
            _ => panic!("not a u32"),
        };
        assert_eq!(values[0], 5408);

        let values = match t.ifd.get(&282u16) {
            Some(TagPayload::F64val(v)) => v,
            _ => panic!("not a f64"),
        };
        assert_eq!(*values, vec![37.7953; 1]);

        let values = match t.ifd.get(&283u16) {
            Some(TagPayload::F64val(v)) => v,
            _ => panic!("not a f64"),
        };
        assert_eq!(*values, vec![37.7953; 1]);

        let values = match t.ifd.get(&296u16) {
            Some(TagPayload::U16val(v)) => v,
            _ => panic!("not a u16"),
        };
        assert_eq!(values[0], 3);

        let values = match t.ifd.get(&338u16) {
            Some(TagPayload::U16val(v)) => v,
            _ => panic!("not a u16"),
        };
        assert_eq!(values[0], 2);

    }

    #[test]
    fn little_endian_tiff() {
        // Scenario:  read a little-endian TIFF with a known
        // string tag
        //
        // Expected result:  tag 305 has value "IrfanView"

        let t = Tiff::new("test-data/LogoFH.tif").unwrap();

        let value = match t.ifd.get(&305u16) {
            Some(TagPayload::Sval(v)) => v,
            _ => panic!("not a string"),
        };
        assert_eq!(value, "IrfanView");

    }

    #[test]
    fn tiff_endian_signature_too_small() {
        // Scenario: the file signature not long enough to read in
        // 
        // Expected result:  error

        let result = Tiff::new("test-data/1byte.tif");
        assert!(result.is_err());
        assert!(matches!(result, Err(TiffError::Io{ .. })));

    }

    #[test]
    fn bad_tiff_version() {
        // Scenario:  the file does not identify as classic TIFF
        //
        // Expected result:  io::ErrorKind::Other
        let result = Tiff::new("test-data/bad-tiff-version.dat");
        assert!(result.is_err());
        assert!(matches!(result, Err(TiffError::Class{ .. })));
    }

    #[test]
    fn bad_endianness() {
        // Scenario:  the does not specify a valid endianness
        //
        // Expected result:  io::ErrorKind::Other
        let result = Tiff::new("Cargo.toml");
        assert!(result.is_err());
        assert!(matches!(result, Err(TiffError::Endian{ .. })));
    }
}
