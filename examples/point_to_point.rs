use log::{error, info, trace, warn};
use std::env::current_dir;
use std::fs::canonicalize;
use std::sync::Arc;
use std::time::Duration;
use protobuf::Enum;
use tokio::sync::Notify;
use up_client_vsomeip_rust::UPClientVsomeip;
use up_rust::{UCode, UListener, UMessage, UMessageBuilder, UMessageType, UStatus, UTransport, UUri};
use up_rust::UMessageType::UMESSAGE_TYPE_UNSPECIFIED;

pub struct PrintingListener {
    client: Arc<UPClientVsomeip>
}

impl PrintingListener {
    pub fn new(client: Arc<UPClientVsomeip>) -> Self {
        Self {
            client
        }
    }
}

#[async_trait::async_trait]
impl UListener for PrintingListener {
    async fn on_receive(&self, msg: UMessage) {
        println!("Received in point-to-point listener:\n{:?}", msg);

        match msg.attributes.type_.enum_value_or(UMESSAGE_TYPE_UNSPECIFIED) {
            UMESSAGE_TYPE_UNSPECIFIED => {
                warn!("Not supported message type: UNSPECIFIED");
                return;
            }
            UMessageType::UMESSAGE_TYPE_PUBLISH => {
                warn!("uProtocol PUBLISH received. This shouldn't happen!");
                panic!();
            }
            UMessageType::UMESSAGE_TYPE_REQUEST => {
                let response_build_res = UMessageBuilder::response_for_request(msg.attributes.as_ref().unwrap()).with_comm_status(UCode::OK.value()).build();
                let Ok(response_msg) = response_build_res else {
                    warn!("Unable to make uProtocol Response message: {:?}", response_build_res.err().unwrap());
                    return;
                };
                let _ = self.client.send(response_msg).await.inspect_err(|err| {
                    warn!("Unable to send response: {err:?}");
                });
                info!("Able to send RESPONSE");
            }
            UMessageType::UMESSAGE_TYPE_RESPONSE => {
                info!("Received RESPONSE");
                return;
            }
            UMessageType::UMESSAGE_TYPE_NOTIFICATION => {
                warn!("Not supported message type: NOTIFICATION");
                return;
            }
        }
    }

    async fn on_error(&self, err: UStatus) {
        println!("{:?}", err);
    }
}

fn any_uuri() -> UUri {
    UUri {
        authority_name: "*".to_string(), // any authority
        ue_id: 0x0000_FFFF,              // any instance, any service
        ue_version_major: 0xFF,          // any
        resource_id: 0xFFFF,             // any
        ..Default::default()
    }
}

fn any_from_authority(authority_name: &str) -> UUri {
    UUri {
        authority_name: authority_name.to_string(),
        ue_id: 0x0000_FFFF,     // any instance, any service
        ue_version_major: 0xFF, // any
        resource_id: 0xFFFF,    // any
        ..Default::default()
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let service_authority_name = "foo";
    let streamer_ue_id = 0x9876;

    // let service_1_ue_id = 0x1234; // TODO: Temporarily commented out just to try sending a Request to service
    let service_1_ue_id = 0x1236;
    let service_1_ue_version_major = 1;
    let service_1_resource_id = 0x0421;

    let current_dir = current_dir();
    println!("{current_dir:?}");

    let vsomeip_config_path = "vsomeip_configs/example_ustreamer.json";
    let abs_vsomeip_config_path = canonicalize(vsomeip_config_path).ok();
    println!("abs_vsomeip_config_path: {abs_vsomeip_config_path:?}");

    let client_res = UPClientVsomeip::new_with_config(
        &service_authority_name.to_string(),
        streamer_ue_id,
        &abs_vsomeip_config_path.unwrap(),
    );
    let Ok(client) = client_res else {
        error!("Unable to establish UTransport");
        return;
    };
    let client = Arc::new(client);

    let source = any_from_authority(service_authority_name);
    let sink = any_uuri();

    let printing_listener: Arc<dyn UListener> = Arc::new(PrintingListener::new(client.clone()));
    let reg_res = client
        .register_listener(&source, Some(&sink), printing_listener)
        .await;
    if reg_res.is_err() {
        error!("Unable to register with UTransport");
    }

    // craft a Request that can be served
    let service_2_ue_id = 0x1235;
    let service_2_ue_version_major = 1;
    let service_2_resource_id = 0x0422;

    let req_sink = UUri {
        authority_name: "foo".to_string(),
        ue_id: service_2_ue_id,
        ue_version_major: service_2_ue_version_major,
        resource_id: service_2_resource_id,
        ..Default::default()
    };

    let req_source = UUri {
        authority_name: "bar".to_string(),
        ue_id: service_1_ue_id,
        ue_version_major: service_1_ue_version_major,
        resource_id: 0x0, // i.e. me
        ..Default::default()
    };

    loop {
        let request_msg_res = UMessageBuilder::request(req_sink.clone(), req_source.clone(), 10000).build().unwrap();
        let send_res = client.send(request_msg_res.clone()).await;

        if let Err(err) = send_res {
            warn!("Unable to send message: {err:?}");
            continue;
        }

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
