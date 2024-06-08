use log::error;
use protobuf::Enum;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::Notify;
use up_client_vsomeip_rust::UPClientVsomeip;
use up_rust::{UCode, UListener, UMessage, UMessageBuilder, UStatus, UTransport, UUri};

pub struct RequestListener {
    client: Arc<UPClientVsomeip>,
}

impl RequestListener {
    pub fn new(client: Arc<UPClientVsomeip>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl UListener for RequestListener {
    async fn on_receive(&self, msg: UMessage) {
        println!("Received Request:\n{:?}", msg);

        let response_msg = UMessageBuilder::response_for_request(&msg.attributes)
            .with_comm_status(UCode::OK.value())
            .build();
        let Ok(response_msg) = response_msg else {
            error!(
                "Unable to create response_msg: {:?}",
                response_msg.err().unwrap()
            );
            return;
        };
        let client = self.client.clone();
        let send_res = client.send(response_msg).await;
        if let Err(err) = send_res {
            error!("Unable to send response_msg: {:?}", err);
        }
    }

    async fn on_error(&self, err: UStatus) {
        println!("{:?}", err);
    }
}

fn any_uuri() -> UUri {
    UUri {
        authority_name: "*".to_string(),
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

    let service_1_ue_id = 0x1234;
    let service_1_ue_version_major = 1;
    let service_1_resource_id = 0x0421;

    let service_2_ue_id = 0x1235;
    let service_2_ue_version_major = 1;
    let service_2_resource_id = 0x0422;

    let client_res = UPClientVsomeip::new(&service_authority_name.to_string(), streamer_ue_id);

    let Ok(client) = client_res else {
        error!("Unable to establish subscriber");
        return;
    };

    let client = Arc::new(client);

    let service_1_uuri = UUri {
        authority_name: service_authority_name.to_string(),
        ue_id: service_1_ue_id as u32,
        ue_version_major: service_1_ue_version_major,
        resource_id: service_1_resource_id,
        ..Default::default()
    };

    let service_2_uuri = UUri {
        authority_name: service_authority_name.to_string(),
        ue_id: service_2_ue_id as u32,
        ue_version_major: service_2_ue_version_major,
        resource_id: service_2_resource_id,
        ..Default::default()
    };

    let printing_listener: Arc<dyn UListener> = Arc::new(RequestListener::new(client.clone()));

    let reg_service_1 = client
        .register_listener(
            &any_uuri(),
            Some(&service_1_uuri),
            printing_listener.clone(),
        )
        .await;

    if let Err(err) = reg_service_1 {
        error!("Unable to register: {:?}", err);
    }

    let reg_service_2 = client
        .register_listener(&any_uuri(), Some(&service_2_uuri), printing_listener)
        .await;

    if let Err(err) = reg_service_2 {
        error!("Unable to register: {:?}", err);
    }

    let notify = Arc::new(Notify::new());
    notify.notified().await;
}
