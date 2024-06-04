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
        println!("Received Response:\n{:?}", msg);
    }

    async fn on_error(&self, err: UStatus) {
        println!("{:?}", err);
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let app_name = "client";

    let service_authority_name = "foo";
    let service_ue_id = 0x1234;
    let service_ue_version_major = 1;
    let service_resource_id = 0x0421;

    let client_authority_name = "bar";
    let client_ue_id = 0x0345;
    let client_ue_version_major = 1;
    let client_resource_id = 0x0000;

    let client_res = UPClientVsomeip::new(client_authority_name, client_ue_id);

    let Ok(client) = client_res else {
        error!("Unable to establish subscriber");
        return;
    };

    let client_uuri = UUri {
        authority_name: client_authority_name.to_string(),
        ue_id: client_ue_id as u32,
        ue_version_major: client_ue_version_major,
        resource_id: client_resource_id,
        ..Default::default()
    };

    let service_uuri = UUri {
        authority_name: service_authority_name.to_string(),
        ue_id: service_ue_id,
        ue_version_major: service_ue_version_major,
        resource_id: service_resource_id,
        ..Default::default()
    };

    let printing_listener: Arc<dyn UListener> = Arc::new(PrintingListener);
    let reg_res = client
        .register_listener(&service_uuri, Some(&client_uuri), printing_listener)
        .await;
    if let Err(err) = reg_res {
        error!("Unable to register for returning Response: {:?}", err);
    }

    loop {
        let request_msg_res =
            UMessageBuilder::request(service_uuri.clone(), client_uuri.clone(), 10000).build();

        let Ok(request_msg) = request_msg_res else {
            error!(
                "Unable to create Request UMessage: {:?}",
                request_msg_res.err().unwrap()
            );
            continue;
        };

        let send_res = client.send(request_msg).await;

        if let Err(err) = send_res {
            error!("Unable to send Request UMessage: {:?}", err);
            continue;
        }

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
