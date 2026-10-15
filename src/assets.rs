//! Static geometry extraction for Recoil S3O models and BOS unit scripts.
//!
//! This runs once while preparing a catalog, never in the simulation hot loop.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Piece {
    emit_origin: [f32; 3],
}

#[derive(Debug)]
pub struct S3oModel {
    pieces: HashMap<String, Piece>,
}

impl S3oModel {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 52 || data.get(..12) != Some(b"Spring unit\0") {
            return Err("not a version-0 S3O model".into());
        }
        if read_i32(data, 12)? != 0 {
            return Err("unsupported S3O version".into());
        }
        let root_offset = read_offset(data, 36)?;
        let mut pieces = HashMap::new();
        let mut visited = Vec::new();
        parse_piece(data, root_offset, [0.0; 3], &mut pieces, &mut visited, 0)?;
        Ok(Self { pieces })
    }

    /// Returns the engine's rest-pose projectile emission position for a piece.
    pub fn emit_origin(&self, piece_name: &str) -> Option<[f32; 3]> {
        self.pieces
            .get(&piece_name.to_ascii_lowercase())
            .map(|piece| piece.emit_origin)
    }
}

pub fn derive_firing_origin(
    s3o_data: &[u8],
    bos_source: &str,
    weapon_number: usize,
) -> Result<[f32; 3], String> {
    let piece_name = query_weapon_piece(bos_source, weapon_number).ok_or_else(|| {
        format!("could not determine QueryWeapon piece for weapon {weapon_number}")
    })?;
    S3oModel::parse(s3o_data)?
        .emit_origin(&piece_name)
        .ok_or_else(|| format!("piece {piece_name:?} is absent from the S3O model"))
}

/// Extracts a simple BOS `QueryWeapon`/legacy `QueryPrimary` return piece.
/// Complex branches intentionally fail closed so an asset compiler can report
/// the unit instead of quietly choosing the wrong barrel.
pub fn query_weapon_piece(source: &str, weapon_number: usize) -> Option<String> {
    let stripped = strip_comments(source);
    let lower = stripped.to_ascii_lowercase();
    let modern = format!("queryweapon{weapon_number}");
    let legacy = match weapon_number {
        1 => Some("queryprimary"),
        2 => Some("querysecondary"),
        3 => Some("querytertiary"),
        _ => None,
    };
    let start = find_function(&lower, &modern)
        .or_else(|| legacy.and_then(|name| find_function(&lower, name)))?;
    let open = lower[start..].find('{')? + start;
    let close = matching_brace(&lower, open)?;
    let body = &stripped[open + 1..close];
    let body_lower = &lower[open + 1..close];
    let return_at = body_lower.find("return")? + "return".len();
    let tail = body[return_at..].trim_start();
    let tail = tail.strip_prefix('(').map_or(tail, str::trim_start);
    let name: String = tail
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn parse_piece(
    data: &[u8],
    offset: usize,
    parent: [f32; 3],
    pieces: &mut HashMap<String, Piece>,
    visited: &mut Vec<usize>,
    depth: usize,
) -> Result<(), String> {
    if depth > 256 || visited.contains(&offset) {
        return Err("cyclic or excessively deep S3O piece tree".into());
    }
    checked_slice(data, offset, 52)?;
    visited.push(offset);
    let name_offset = read_offset(data, offset)?;
    let child_count = read_count(data, offset + 4)?;
    let children_offset = read_offset(data, offset + 8)?;
    let vertex_count = read_count(data, offset + 12)?;
    let vertex_offset = read_offset(data, offset + 16)?;
    let local = [
        read_f32(data, offset + 40)?,
        read_f32(data, offset + 44)?,
        read_f32(data, offset + 48)?,
    ];
    let absolute = add3(parent, local);
    let emit_origin = if vertex_count > 1 {
        add3(
            absolute,
            [
                read_f32(data, vertex_offset)?,
                read_f32(data, vertex_offset + 4)?,
                read_f32(data, vertex_offset + 8)?,
            ],
        )
    } else {
        absolute
    };
    let name = read_c_string(data, name_offset)?.to_ascii_lowercase();
    if pieces.insert(name.clone(), Piece { emit_origin }).is_some() {
        return Err(format!("duplicate S3O piece name {name:?}"));
    }
    checked_slice(data, children_offset, child_count.saturating_mul(4))?;
    for child in 0..child_count {
        let child_offset = read_offset(data, children_offset + child * 4)?;
        parse_piece(data, child_offset, absolute, pieces, visited, depth + 1)?;
    }
    visited.pop();
    Ok(())
}

fn find_function(source: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(relative) = source[from..].find(name) {
        let at = from + relative;
        let before_ok = at == 0
            || !source.as_bytes()[at - 1].is_ascii_alphanumeric()
                && source.as_bytes()[at - 1] != b'_';
        let after = at + name.len();
        let after_ok = after >= source.len()
            || !source.as_bytes()[after].is_ascii_alphanumeric()
                && source.as_bytes()[after] != b'_';
        if before_ok && after_ok {
            return Some(at);
        }
        from = after;
    }
    None
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (relative, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + relative);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                output.push(' ');
                index += 1;
            }
        } else if bytes[index..].starts_with(b"/*") {
            output.push_str("  ");
            index += 2;
            while index < bytes.len() && !bytes[index..].starts_with(b"*/") {
                output.push(if bytes[index] == b'\n' { '\n' } else { ' ' });
                index += 1;
            }
            if index < bytes.len() {
                output.push_str("  ");
                index += 2;
            }
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn checked_slice(data: &[u8], offset: usize, size: usize) -> Result<&[u8], String> {
    data.get(offset..offset.saturating_add(size))
        .ok_or_else(|| "truncated S3O data".into())
}

fn read_i32(data: &[u8], offset: usize) -> Result<i32, String> {
    let bytes: [u8; 4] = checked_slice(data, offset, 4)?.try_into().unwrap();
    Ok(i32::from_le_bytes(bytes))
}

fn read_offset(data: &[u8], offset: usize) -> Result<usize, String> {
    usize::try_from(read_i32(data, offset)?).map_err(|_| "negative S3O offset".into())
}

fn read_count(data: &[u8], offset: usize) -> Result<usize, String> {
    usize::try_from(read_i32(data, offset)?).map_err(|_| "negative S3O count".into())
}

fn read_f32(data: &[u8], offset: usize) -> Result<f32, String> {
    let bytes: [u8; 4] = checked_slice(data, offset, 4)?.try_into().unwrap();
    Ok(f32::from_le_bytes(bytes))
}

fn read_c_string(data: &[u8], offset: usize) -> Result<String, String> {
    let tail = data.get(offset..).ok_or("invalid S3O string offset")?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("unterminated S3O string")?;
    std::str::from_utf8(&tail[..end])
        .map(str::to_owned)
        .map_err(|_| "non-UTF-8 S3O piece name".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_i32(data: &mut [u8], offset: usize, value: i32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put_f32(data: &mut [u8], offset: usize, value: f32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn resolves_query_piece_and_s3o_emit_vertex() {
        let piece_offset = 64;
        let name_offset = piece_offset + 52;
        let vertex_offset = name_offset + 6;
        let mut data = vec![0_u8; vertex_offset + 64];
        data[..12].copy_from_slice(b"Spring unit\0");
        put_i32(&mut data, 36, piece_offset as i32);
        put_i32(&mut data, piece_offset, name_offset as i32);
        put_i32(&mut data, piece_offset + 12, 2);
        put_i32(&mut data, piece_offset + 16, vertex_offset as i32);
        put_f32(&mut data, piece_offset + 40, 1.0);
        put_f32(&mut data, piece_offset + 44, 2.0);
        put_f32(&mut data, piece_offset + 48, 3.0);
        data[name_offset..name_offset + 6].copy_from_slice(b"flare\0");
        put_f32(&mut data, vertex_offset, 4.0);
        put_f32(&mut data, vertex_offset + 4, 5.0);
        put_f32(&mut data, vertex_offset + 8, 6.0);

        let script = "QueryPrimary() { // return(fake);\n return (flare); }";
        assert_eq!(
            derive_firing_origin(&data, script, 1).unwrap(),
            [5.0, 7.0, 9.0]
        );
    }

    #[test]
    fn parses_modern_query_weapon_name() {
        assert_eq!(
            query_weapon_piece("QueryWeapon2() { return barrel_tip; }", 2).as_deref(),
            Some("barrel_tip")
        );
    }
}
