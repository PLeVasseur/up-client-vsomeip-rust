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

    let service_authority_name = "foo";
    let streamer_ue_id = 0x7878;

    let service_1_ue_id = 0x1234;
    let service_1_ue_version_major = 1;
    let service_1_resource_id = 0x0421;

    let service_2_ue_id = 0x1235;
    let service_2_ue_version_major = 1;
    let service_2_resource_id = 0x0422;

    let client_authority_name = "bar";
    let client_ue_id = 0x0345;
    let client_ue_version_major = 1;
    let client_resource_id = 0x0000;

    let client_res = UPClientVsomeip::new(&client_authority_name.to_string(), streamer_ue_id);

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

    let printing_listener: Arc<dyn UListener> = Arc::new(PrintingListener);
    let reg_res_1 = client
        .register_listener(
            &service_1_uuri,
            Some(&client_uuri),
            printing_listener.clone(),
        )
        .await;
    if let Err(err) = reg_res_1 {
        error!("Unable to register for returning Response: {:?}", err);
    }

    let reg_res_1 = client
        .register_listener(&service_2_uuri, Some(&client_uuri), printing_listener)
        .await;
    if let Err(err) = reg_res_1 {
        error!("Unable to register for returning Response: {:?}", err);
    }

    loop {
        let request_msg_res_1 =
            UMessageBuilder::request(service_1_uuri.clone(), client_uuri.clone(), 10000).build();

        let Ok(request_msg_1) = request_msg_res_1 else {
            error!(
                "Unable to create Request UMessage: {:?}",
                request_msg_res_1.err().unwrap()
            );
            continue;
        };

        let send_res_1 = client.send(request_msg_1).await;

        if let Err(err) = send_res_1 {
            error!("Unable to send Request UMessage: {:?}", err);
            continue;
        }

        let request_msg_res_2 =
            UMessageBuilder::request(service_2_uuri.clone(), client_uuri.clone(), 10000).build();

        let Ok(request_msg_2) = request_msg_res_2 else {
            error!(
                "Unable to create Request UMessage: {:?}",
                request_msg_res_2.err().unwrap()
            );
            continue;
        };

        let send_res_2 = client.send(request_msg_2).await;

        if let Err(err) = send_res_2 {
            error!("Unable to send Request UMessage: {:?}", err);
            continue;
        }

        tokio::time::sleep(Duration::from_millis(2000)).await;
    }
}
