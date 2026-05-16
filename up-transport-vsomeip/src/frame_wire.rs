/********************************************************************************
 * Copyright (c) 2026 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/

use bytes::Bytes;
use up_rust::{
    UAttributes, UCode, UEncoding, UFrameMetadata, UMessageType, UOwnedFrame, UPriority, UStatus,
    UUri, UUID,
};

const FRAME_PAYLOAD_MAGIC: &[u8; 4] = b"USIP";
const FRAME_PAYLOAD_VERSION: u8 = 1;

pub(crate) fn encode_frame_payload(frame: &UOwnedFrame) -> Result<Vec<u8>, UStatus> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(FRAME_PAYLOAD_MAGIC);
    bytes.push(FRAME_PAYLOAD_VERSION);
    write_u64(&mut bytes, frame.metadata().attributes().id().msb());
    write_u64(&mut bytes, frame.metadata().attributes().id().lsb());
    bytes.push(message_type_to_byte(
        frame.metadata().attributes().message_type(),
    ));
    bytes.push(priority_to_byte(frame.metadata().attributes().priority()));
    write_optional_u32(&mut bytes, frame.metadata().attributes().ttl());
    append_string(
        &mut bytes,
        &frame.metadata().attributes().source().to_uri(false),
    )?;
    append_string(
        &mut bytes,
        frame
            .metadata()
            .attributes()
            .sink()
            .map(|uri| uri.to_uri(false))
            .as_deref()
            .unwrap_or_default(),
    )?;
    append_string(&mut bytes, frame.metadata().encoding().format_id())?;
    append_string(&mut bytes, frame.metadata().encoding().content_type())?;
    append_string(
        &mut bytes,
        frame.metadata().encoding().schema_ref().unwrap_or_default(),
    )?;
    write_optional_uuid(&mut bytes, frame.metadata().attributes().request_id());
    write_optional_string(&mut bytes, frame.metadata().attributes().traceparent())?;
    write_optional_string(&mut bytes, frame.metadata().attributes().token())?;
    write_optional_u32(&mut bytes, frame.metadata().attributes().permission_level());
    write_optional_code(&mut bytes, frame.metadata().attributes().commstatus());
    bytes.extend_from_slice(frame.payload_bytes());
    Ok(bytes)
}

pub(crate) fn decode_frame_payload(payload: Vec<u8>) -> Result<UOwnedFrame, UStatus> {
    let mut bytes = payload.as_slice();
    let magic = take_bytes(&mut bytes, FRAME_PAYLOAD_MAGIC.len())?;
    if magic != FRAME_PAYLOAD_MAGIC {
        return Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid SOME/IP native frame payload",
        ));
    }
    let version = take_u8(&mut bytes)?;
    if version != FRAME_PAYLOAD_VERSION {
        return Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            format!("unsupported SOME/IP native frame payload version: {version}"),
        ));
    }

    let id = UUID::from_u64_pair(take_u64(&mut bytes)?, take_u64(&mut bytes)?)
        .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;
    let message_type = byte_to_message_type(take_u8(&mut bytes)?)?;
    let priority = byte_to_priority(take_u8(&mut bytes)?)?;
    let ttl = take_optional_u32(&mut bytes)?;
    let source = UUri::try_from(take_string(&mut bytes)?.as_str()).map_err(|e| {
        UStatus::fail_with_code(UCode::INVALID_ARGUMENT, format!("invalid source URI: {e}"))
    })?;
    let sink = {
        let sink = take_string(&mut bytes)?;
        if sink.is_empty() {
            None
        } else {
            Some(UUri::try_from(sink.as_str()).map_err(|e| {
                UStatus::fail_with_code(UCode::INVALID_ARGUMENT, format!("invalid sink URI: {e}"))
            })?)
        }
    };
    let format_id = take_string(&mut bytes)?;
    let content_type = take_string(&mut bytes)?;
    let schema_ref = take_string(&mut bytes)?;
    let schema_ref = if schema_ref.is_empty() {
        None
    } else {
        Some(schema_ref)
    };
    let request_id = take_optional_uuid(&mut bytes)?;
    let traceparent = take_optional_string(&mut bytes)?;
    let token = take_optional_string(&mut bytes)?;
    let permission_level = take_optional_u32(&mut bytes)?;
    let commstatus = take_optional_code(&mut bytes)?;

    let mut attributes = UAttributes::new(id, source, sink, message_type).with_priority(priority);
    if let Some(ttl) = ttl {
        attributes = attributes.with_ttl(ttl);
    }
    if let Some(request_id) = request_id {
        attributes = attributes.with_request_id(request_id);
    }
    if let Some(traceparent) = traceparent {
        attributes = attributes.with_traceparent(traceparent);
    }
    if let Some(token) = token {
        attributes = attributes.with_token(token);
    }
    if let Some(permission_level) = permission_level {
        attributes = attributes.with_permission_level(permission_level);
    }
    if let Some(commstatus) = commstatus {
        attributes = attributes.with_comm_status(commstatus);
    }

    Ok(UOwnedFrame::new(
        UFrameMetadata::new(
            attributes,
            UEncoding::new(format_id, content_type, schema_ref),
        ),
        Bytes::copy_from_slice(bytes),
    ))
}

fn write_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn write_optional_u32(dst: &mut Vec<u8>, value: Option<u32>) {
    match value {
        Some(value) => {
            dst.push(1);
            dst.extend_from_slice(&value.to_le_bytes());
        }
        None => dst.push(0),
    }
}

fn write_optional_uuid(dst: &mut Vec<u8>, value: Option<&UUID>) {
    match value {
        Some(value) => {
            dst.push(1);
            write_u64(dst, value.msb());
            write_u64(dst, value.lsb());
        }
        None => dst.push(0),
    }
}

fn write_optional_string(dst: &mut Vec<u8>, value: Option<&str>) -> Result<(), UStatus> {
    match value {
        Some(value) => {
            dst.push(1);
            append_string(dst, value)?;
        }
        None => dst.push(0),
    }
    Ok(())
}

fn write_optional_code(dst: &mut Vec<u8>, value: Option<UCode>) {
    match value {
        Some(value) => {
            dst.push(1);
            dst.push(value.as_u8());
        }
        None => dst.push(0),
    }
}

fn append_string(dst: &mut Vec<u8>, value: &str) -> Result<(), UStatus> {
    let len = u32::try_from(value.len()).map_err(|_| {
        UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "frame metadata field is too large")
    })?;
    dst.extend_from_slice(&len.to_le_bytes());
    dst.extend_from_slice(value.as_bytes());
    Ok(())
}

fn take_u8(src: &mut &[u8]) -> Result<u8, UStatus> {
    let (value, remaining) = src
        .split_first()
        .ok_or_else(|| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid frame payload"))?;
    *src = remaining;
    Ok(*value)
}

fn take_u64(src: &mut &[u8]) -> Result<u64, UStatus> {
    let bytes = take_bytes(src, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
        UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid frame payload")
    })?))
}

fn take_optional_u32(src: &mut &[u8]) -> Result<Option<u32>, UStatus> {
    match take_u8(src)? {
        0 => Ok(None),
        1 => {
            let bytes = take_bytes(src, 4)?;
            Ok(Some(u32::from_le_bytes(bytes.try_into().map_err(
                |_| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid frame payload"),
            )?)))
        }
        _ => Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid optional value",
        )),
    }
}

fn take_optional_uuid(src: &mut &[u8]) -> Result<Option<UUID>, UStatus> {
    match take_u8(src)? {
        0 => Ok(None),
        1 => UUID::from_u64_pair(take_u64(src)?, take_u64(src)?)
            .map(Some)
            .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string())),
        _ => Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid optional value",
        )),
    }
}

fn take_optional_string(src: &mut &[u8]) -> Result<Option<String>, UStatus> {
    match take_u8(src)? {
        0 => Ok(None),
        1 => Ok(Some(take_string(src)?)),
        _ => Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid optional value",
        )),
    }
}

fn take_optional_code(src: &mut &[u8]) -> Result<Option<UCode>, UStatus> {
    match take_u8(src)? {
        0 => Ok(None),
        1 => UCode::from_u8(take_u8(src)?)
            .map(Some)
            .ok_or_else(|| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid status code")),
        _ => Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid optional value",
        )),
    }
}

fn take_string(src: &mut &[u8]) -> Result<String, UStatus> {
    let len_bytes = take_bytes(src, 4)?;
    let len = usize::try_from(u32::from_le_bytes(len_bytes.try_into().map_err(|_| {
        UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid frame payload")
    })?))
    .map_err(|e| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, e.to_string()))?;
    let bytes = take_bytes(src, len)?;
    String::from_utf8(bytes.to_vec()).map_err(|e| {
        UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            format!("frame metadata field is not valid UTF-8: {e}"),
        )
    })
}

fn take_bytes<'a>(src: &mut &'a [u8], len: usize) -> Result<&'a [u8], UStatus> {
    let value = src
        .get(..len)
        .ok_or_else(|| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid frame payload"))?;
    *src = src
        .get(len..)
        .ok_or_else(|| UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "invalid frame payload"))?;
    Ok(value)
}

fn message_type_to_byte(message_type: UMessageType) -> u8 {
    match message_type {
        UMessageType::Publish => 1,
        UMessageType::Notification => 2,
        UMessageType::Request => 3,
        UMessageType::Response => 4,
    }
}

fn byte_to_message_type(value: u8) -> Result<UMessageType, UStatus> {
    match value {
        1 => Ok(UMessageType::Publish),
        2 => Ok(UMessageType::Notification),
        3 => Ok(UMessageType::Request),
        4 => Ok(UMessageType::Response),
        _ => Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid message type",
        )),
    }
}

fn priority_to_byte(priority: UPriority) -> u8 {
    match priority {
        UPriority::CS0 => 0,
        UPriority::CS1 => 1,
        UPriority::CS2 => 2,
        UPriority::CS3 => 3,
        UPriority::CS4 => 4,
        UPriority::CS5 => 5,
        UPriority::CS6 => 6,
    }
}

fn byte_to_priority(value: u8) -> Result<UPriority, UStatus> {
    match value {
        0 => Ok(UPriority::CS0),
        1 => Ok(UPriority::CS1),
        2 => Ok(UPriority::CS2),
        3 => Ok(UPriority::CS3),
        4 => Ok(UPriority::CS4),
        5 => Ok(UPriority::CS5),
        6 => Ok(UPriority::CS6),
        _ => Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "invalid priority",
        )),
    }
}

#[cfg(test)]
mod tests {
    use protobuf::well_known_types::wrappers::StringValue;
    use up_rust::{ProtobufWire, WireFormat};

    use super::*;

    #[test]
    fn frame_payload_round_trips_metadata_and_payload() {
        let source = UUri::try_from("//vehicle/A8000/2/8001").unwrap();
        let sink = UUri::try_from("//service/B8000/1/0").unwrap();
        let request_id = UUID::build();
        let attributes =
            UAttributes::new(UUID::build(), source, Some(sink), UMessageType::Response)
                .with_priority(UPriority::CS4)
                .with_ttl(1234)
                .with_request_id(request_id)
                .with_traceparent("traceparent")
                .with_token("token")
                .with_permission_level(9)
                .with_comm_status(UCode::UNAVAILABLE);
        let frame = UOwnedFrame::new(
            UFrameMetadata::new(
                attributes,
                UEncoding::new("custom", "application/custom", Some("schema://custom")),
            ),
            [1_u8, 2, 3, 4].as_slice(),
        );

        let encoded = encode_frame_payload(&frame).unwrap();
        let decoded = decode_frame_payload(encoded).unwrap();

        assert_eq!(decoded, frame);
    }

    #[test]
    fn rejects_invalid_frame_payload() {
        let error = decode_frame_payload(vec![1, 2, 3]).unwrap_err();

        assert_eq!(error.get_code(), UCode::INVALID_ARGUMENT);
    }

    #[test]
    fn rejects_invalid_optional_marker_in_metadata() {
        let source = UUri::try_from("//vehicle/A8000/2/8001").unwrap();
        let frame = UOwnedFrame::new(UFrameMetadata::publish(source), [1_u8, 2, 3].as_slice());
        let mut encoded = encode_frame_payload(&frame).unwrap();
        let ttl_marker_index = FRAME_PAYLOAD_MAGIC.len() + 1 + 8 + 8 + 1 + 1;
        *encoded
            .get_mut(ttl_marker_index)
            .expect("encoded frame contains TTL marker") = 2;

        let error = decode_frame_payload(encoded).unwrap_err();

        assert_eq!(error.get_code(), UCode::INVALID_ARGUMENT);
    }

    #[test]
    fn preserves_expired_ttl_metadata_for_delivery_layer() {
        let expired_id = UUID::from_u64_pair(0x018D_548E_A8E0_7000, 0x8000_0000_0000_0000)
            .expect("valid expired UUID");
        let source = UUri::try_from("//vehicle/A8000/2/8001").unwrap();
        let attributes = UAttributes::new(expired_id, source, None, UMessageType::Publish)
            .with_priority(UPriority::CS1)
            .with_ttl(1);
        let frame = UOwnedFrame::new(
            UFrameMetadata::new(
                attributes,
                UEncoding::without_schema_ref("raw", "application/octet-stream"),
            ),
            [1_u8, 2, 3].as_slice(),
        );

        let decoded = decode_frame_payload(encode_frame_payload(&frame).unwrap()).unwrap();

        assert!(decoded.metadata().attributes().is_expired());
    }

    #[test]
    fn frame_payload_preserves_protobuf_payload_as_payload_only() {
        let source = UUri::try_from("//vehicle/A8000/2/8001").unwrap();
        let mut value = StringValue::new();
        value.value = "protobuf payload".to_string();
        let frame = UOwnedFrame::from_serializable::<ProtobufWire, _>(
            UFrameMetadata::publish(source),
            &value,
        )
        .unwrap();

        let encoded = encode_frame_payload(&frame).unwrap();
        let decoded = decode_frame_payload(encoded).unwrap();
        let decoded_payload: StringValue = decoded.deserialize::<ProtobufWire, _>().unwrap();

        assert_eq!(decoded.metadata().encoding(), &ProtobufWire::encoding());
        assert_eq!(decoded_payload.value, value.value);
    }
}
