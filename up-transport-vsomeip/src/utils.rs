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

/// Splits a uProtocol entity ID into SOME/IP InstanceID and ServiceID.
///
/// A zero upper half represents the SOME/IP default InstanceID `1`.
pub fn split_ue_id_to_instance_service(ue_id: UeId) -> (u16, u16) {
    let (instance_id, service_id) = split_u32_to_u16(ue_id);
    (instance_id.max(1), service_id)
}

pub fn create_ue_id(instance_id: u16, service_id: u16) -> UeId {
    ((instance_id as UeId) << 16) | service_id as UeId
}

/// Create a vsomeip request_id from client_id and session_id as per SOME/IP spec
pub fn create_request_id(client_id: ClientId, session_id: SessionId) -> SomeIpRequestId {
    ((client_id as u32) << 16) | (session_id as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_ue_id_to_instance_service_defaults_zero_instance_to_one() {
        assert_eq!(split_ue_id_to_instance_service(0x0000_abcd), (1, 0xabcd));
    }

    #[test]
    fn split_ue_id_to_instance_service_preserves_explicit_instance() {
        assert_eq!(split_ue_id_to_instance_service(0x0002_abcd), (2, 0xabcd));
    }

    #[test]
    fn create_ue_id_combines_instance_and_service() {
        assert_eq!(create_ue_id(0x0002, 0xabcd), 0x0002_abcd);
    }
}
