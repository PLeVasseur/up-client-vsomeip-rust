use log::error;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use protobuf::Enum;
use tokio::sync::Notify;
use up_client_vsomeip_rust::UPClientVsomeip;
use up_rust::{UCode, UListener, UMessage, UMessageBuilder, UStatus, UTransport, UUri};

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
        println!("Received Request:\n{:?}", msg);

        let response_msg = UMessageBuilder::response_for_request(&msg.attributes).with_comm_status(UCode::OK.value()).build();
        let Ok(response_msg) = response_msg else {
            error!("Unable to create response_msg: {:?}", response_msg.err().unwrap());
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
        ue_id: 0x0000_FFFF,               // any instance, any service
        ue_version_major: 0xFF,           // any
        resource_id: 0xFFFF,              // any
        ..Default::default()
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let app_name = "service";

    let service_authority_name = "foo";
    let service_ue_id = 0x1234;
    let service_ue_version_major = 1;
    let service_resource_id = 0x0421;

    let client_res = UPClientVsomeip::new(app_name, service_authority_name, service_ue_id);

    let Ok(client) = client_res else {
        error!("Unable to establish subscriber");
        return;
    };

    let client = Arc::new(client);

    let service_uuri = UUri {
        authority_name: service_authority_name.to_string(),
        ue_id: service_ue_id as u32,
        ue_version_major: service_ue_version_major,
        resource_id: service_resource_id,
        ..Default::default()
    };

    let printing_listener: Arc<dyn UListener> = Arc::new(PrintingListener::new(client.clone()));

    let reg_res = client
        .register_listener(&any_uuri(), Some(&service_uuri), printing_listener)
        .await;

    if let Err(err) = reg_res {
        error!("Unable to register: {:?}", err);
    }

    let notify = Arc::new(Notify::new());
    notify.notified().await;
}
