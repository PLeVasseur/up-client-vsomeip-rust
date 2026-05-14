use crate::{AuthorityName, UeId};
use up_rust::UUri;

/// Creates a [UUri] with specified [UUri::authority_name] and [UUri::ue_id]
pub fn any_uuri_fixed_authority_id(authority_name: &AuthorityName, ue_id: UeId) -> UUri {
    UUri {
        authority_name: authority_name.to_string(),
        ue_id,
        ue_version_major: 0xFF, // any
        resource_id: 0xFFFF,    // any
    }
}

/// Useful for splitting u32 into u16s when manipulating [UUri] elements
pub fn split_u32_to_u16(value: u32) -> (u16, u16) {
    let most_significant_bits = (value >> 16) as u16;
    let least_significant_bits = (value & 0xFFFF) as u16;
    (most_significant_bits, least_significant_bits)
}

/// Useful for splitting u32 into u8s when manipulating [UUri] elements
pub fn split_u32_to_u8(value: u32) -> (u8, u8, u8, u8) {
    let byte1 = (value >> 24) as u8;
    let byte2 = ((value >> 16) & 0xFF) as u8;
    let byte3 = ((value >> 8) & 0xFF) as u8;
    let byte4 = (value & 0xFF) as u8;
    (byte1, byte2, byte3, byte4)
}
