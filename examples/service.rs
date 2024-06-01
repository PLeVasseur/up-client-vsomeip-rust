use log::error;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::Notify;
use up_client_vsomeip_rust::UPClientVsomeip;
use up_rust::{UListener, UMessage, UMessageBuilder, UStatus, UTransport, UUri};

pub struct PrintingListener;
#[async_trait::async_trait]
impl UListener for PrintingListener {
    async fn on_receive(&self, msg: UMessage) {
        println!("{:?}", msg);
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

    let service_uuri = UUri {
        authority_name: service_authority_name.to_string(),
        ue_id: service_ue_id as u32,
        ue_version_major: service_ue_version_major,
        resource_id: service_resource_id,
        ..Default::default()
    };

    let printing_listener: Arc<dyn UListener> = Arc::new(PrintingListener);

    let reg_res = client
        .register_listener(&any_uuri(), Some(&service_uuri), printing_listener)
        .await;

    if let Err(err) = reg_res {
        error!("Unable to register: {:?}", err);
    }

    let notify = Arc::new(Notify::new());
    notify.notified().await;
}
