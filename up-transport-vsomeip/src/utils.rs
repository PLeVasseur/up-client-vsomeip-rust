use crate::{AuthorityName, ClientId, InstanceId, ServiceId, SessionId, SomeIpRequestId, UeId};
use up_rust::UUri;

/// Creates a [UUri] with specified [UUri::authority_name] and [UUri::ue_id]
pub fn any_uuri_fixed_authority_id(authority_name: &AuthorityName, ue_id: UeId) -> UUri {
    UUri::try_from_parts(authority_name, ue_id, 0xFF, 0xFFFF)
        .expect("stored local authority must remain valid")
}

/// Reconstructs the raw uEntity identifier from its public UUri components.
pub fn uuri_ue_id(uri: &UUri) -> UeId {
    (u32::from(uri.uentity_instance_id()) << 16) | u32::from(uri.uentity_type_id())
}

/// Splits a uEntity identifier into its SOME/IP instance and service IDs.
pub fn split_ue_id_to_instance_service(ue_id: UeId) -> (InstanceId, ServiceId) {
    let (instance_id, service_id) = split_u32_to_u16(ue_id);
    (instance_id.max(1), service_id)
}

/// Combines SOME/IP instance and service IDs into a uEntity identifier.
pub fn create_ue_id_from_instance_service(instance_id: InstanceId, service_id: ServiceId) -> UeId {
    if instance_id == 1 {
        u32::from(service_id)
    } else {
        (u32::from(instance_id) << 16) | u32::from(service_id)
    }
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
    fn create_ue_id_from_instance_service_compacts_only_default_instance() {
        assert_eq!(create_ue_id_from_instance_service(1, 0xabcd), 0xabcd);
        assert_eq!(create_ue_id_from_instance_service(2, 0xabcd), 0x0002_abcd);
    }
}
