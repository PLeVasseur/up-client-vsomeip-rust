use crate::{AuthorityName, ClientId, SessionId, SomeIpRequestId, UeId};
use up_rust::UUri;

/// Creates a [UUri] with specified [UUri::authority_name] and [UUri::ue_id]
pub fn any_uuri_fixed_authority_id(authority_name: &AuthorityName, ue_id: UeId) -> UUri {
    UUri::try_from_parts(authority_name, ue_id, 0xFF, 0xFFFF)
        .expect("fixed authority wildcard URI must be valid")
}

pub fn uuri_ue_id(uri: &UUri) -> UeId {
    (u32::from(uri.uentity_instance_id()) << 16) | u32::from(uri.uentity_type_id())
}

/// Useful for splitting u32 into u16s when manipulating [UUri] elements
pub fn split_u32_to_u16(value: u32) -> (u16, u16) {
    let most_significant_bits = (value >> 16) as u16;
    let least_significant_bits = (value & 0xFFFF) as u16;
    (most_significant_bits, least_significant_bits)
}

/// Create a vsomeip request_id from client_id and session_id as per SOME/IP spec
pub fn create_request_id(client_id: ClientId, session_id: SessionId) -> SomeIpRequestId {
    ((client_id as u32) << 16) | (session_id as u32)
}
