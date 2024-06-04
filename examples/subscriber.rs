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

#[tokio::main]
async fn main() {
    env_logger::init();

    let app_name = "subscriber";
    let authority_name = "foo";
    let ue_id = 20;

    let client_res = UPClientVsomeip::new(&authority_name.to_string(), ue_id);

    let Ok(client) = client_res else {
        error!("Unable to establish subscriber");
        return;
    };

    let subscription_ue_id = 10;
    let subscription_ue_version_major = 1;
    let subscripton_resource_id = 0x8001;

    let subscriber_topic = UUri {
        authority_name: authority_name.to_string(),
        ue_id: subscription_ue_id,
        ue_version_major: subscription_ue_version_major,
        resource_id: subscripton_resource_id,
        ..Default::default()
    };

    let printing_listener: Arc<dyn UListener> = Arc::new(PrintingListener);

    let reg_res = client
        .register_listener(&subscriber_topic, None, printing_listener)
        .await;

    if let Err(err) = reg_res {
        error!("Unable to register: {:?}", err);
    }

    let notify = Arc::new(Notify::new());
    notify.notified().await;
}
