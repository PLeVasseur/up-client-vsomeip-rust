#[cfg(test)]
mod tests {

    use std::fs::canonicalize;
    use std::sync::Arc;
    use std::time::Duration;
    use log::error;
    use protobuf::Enum;
    use up_rust::{UCode, UListener, UMessage, UMessageBuilder, UStatus, UTransport, UUri};
    use up_client_vsomeip_rust::UPClientVsomeip;

    pub struct ResponseListener;
    #[async_trait::async_trait]
    impl UListener for ResponseListener {
        async fn on_receive(&self, msg: UMessage) {
            println!("Received Response:\n{:?}", msg);
        }

        async fn on_error(&self, err: UStatus) {
            println!("{:?}", err);
        }
    }
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

    #[tokio::test]
    async fn client_service() {
        env_logger::init();

        let service_authority_name = "foo";
        let streamer_ue_id = 0x7878;

        let service_1_ue_id = 0x1234;
        let service_1_ue_version_major = 1;
        let service_1_resource_id_a = 0x0421;
        let service_1_resource_id_b = 0x0422;

        let service_2_ue_id = 0x1235;
        let service_2_ue_version_major = 1;
        let service_2_resource_id = 0x0422;

        let service_3_ue_id = 0x1236;
        let service_3_ue_version_major = 1;
        let service_3_resource_id = 0x0422;

        let client_authority_name = "bar";
        let client_ue_id = 0x0345;
        let client_ue_version_major = 1;
        let client_resource_id = 0x0000;

        let vsomeip_config_path = "vsomeip_configs/client.json";
        let abs_vsomeip_config_path = canonicalize(vsomeip_config_path).ok();
        println!("abs_vsomeip_config_path: {abs_vsomeip_config_path:?}");

        let client_res = UPClientVsomeip::new_with_config(
            &client_authority_name.to_string(),
            streamer_ue_id,
            &abs_vsomeip_config_path.unwrap(),
        );

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

        let service_1_uuri_method_a = UUri {
            authority_name: service_authority_name.to_string(),
            ue_id: service_1_ue_id as u32,
            ue_version_major: service_1_ue_version_major,
            resource_id: service_1_resource_id_a,
            ..Default::default()
        };

        let service_1_uuri_method_b = UUri {
            authority_name: service_authority_name.to_string(),
            ue_id: service_1_ue_id as u32,
            ue_version_major: service_1_ue_version_major,
            resource_id: service_1_resource_id_b,
            ..Default::default()
        };

        let service_2_uuri = UUri {
            authority_name: service_authority_name.to_string(),
            ue_id: service_2_ue_id as u32,
            ue_version_major: service_2_ue_version_major,
            resource_id: service_2_resource_id,
            ..Default::default()
        };

        let service_3_uuri = UUri {
            authority_name: service_authority_name.to_string(),
            ue_id: service_3_ue_id as u32,
            ue_version_major: service_3_ue_version_major,
            resource_id: service_3_resource_id,
            ..Default::default()
        };

        let response_listener: Arc<dyn UListener> = Arc::new(ResponseListener);

        let reg_res_1 = client
            .register_listener(
                &service_1_uuri_method_a,
                Some(&client_uuri),
                response_listener.clone(),
            )
            .await;
        if let Err(err) = reg_res_1 {
            error!("Unable to register for returning Response: {:?}", err);
        }

        let reg_res_1 = client
            .register_listener(
                &service_2_uuri,
                Some(&client_uuri),
                response_listener.clone(),
            )
            .await;
        if let Err(err) = reg_res_1 {
            error!("Unable to register for returning Response: {:?}", err);
        }

        let reg_res_3 = client
            .register_listener(
                &service_3_uuri,
                Some(&client_uuri),
                response_listener.clone(),
            )
            .await;
        if let Err(err) = reg_res_3 {
            error!("Unable to register for returning Response: {:?}", err);
        }

        let service_res = UPClientVsomeip::new(&service_authority_name.to_string(), streamer_ue_id);

        let Ok(service) = service_res else {
            error!("Unable to establish subscriber");
            return;
        };

        let service = Arc::new(service);

        let service_1_uuri = UUri {
            authority_name: service_authority_name.to_string(),
            ue_id: service_1_ue_id as u32,
            ue_version_major: service_1_ue_version_major,
            resource_id: service_1_resource_id_a,
            ..Default::default()
        };

        let service_2_uuri = UUri {
            authority_name: service_authority_name.to_string(),
            ue_id: service_2_ue_id as u32,
            ue_version_major: service_2_ue_version_major,
            resource_id: service_2_resource_id,
            ..Default::default()
        };

        let request_listener: Arc<dyn UListener> = Arc::new(RequestListener::new(service.clone()));

        let reg_service_1 = service
            .register_listener(
                &any_uuri(),
                Some(&service_1_uuri),
                request_listener.clone(),
            )
            .await;

        if let Err(err) = reg_service_1 {
            error!("Unable to register: {:?}", err);
        }

        let reg_service_2 = service
            .register_listener(&any_uuri(), Some(&service_2_uuri), request_listener)
            .await;

        if let Err(err) = reg_service_2 {
            error!("Unable to register: {:?}", err);
        }

        loop {
            let request_msg_res_1_a =
                UMessageBuilder::request(service_1_uuri_method_a.clone(), client_uuri.clone(), 10000)
                    .build();

            let Ok(request_msg_1_a) = request_msg_res_1_a else {
                error!(
                "Unable to create Request UMessage: {:?}",
                request_msg_res_1_a.err().unwrap()
            );
                continue;
            };

            let send_res_1_a = client.send(request_msg_1_a).await;

            if let Err(err) = send_res_1_a {
                error!("Unable to send Request UMessage: {:?}", err);
                continue;
            }

            let request_msg_res_1_b =
                UMessageBuilder::request(service_1_uuri_method_b.clone(), client_uuri.clone(), 10000)
                    .build();

            let Ok(request_msg_1_b) = request_msg_res_1_b else {
                error!(
                "Unable to create Request UMessage: {:?}",
                request_msg_res_1_b.err().unwrap()
            );
                continue;
            };

            let send_res_1_b = client.send(request_msg_1_b).await;

            if let Err(err) = send_res_1_b {
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

            let request_msg_res_3 =
                UMessageBuilder::request(service_3_uuri.clone(), client_uuri.clone(), 10000).build();

            let Ok(request_msg_3) = request_msg_res_3 else {
                error!(
                "Unable to create Request UMessage: {:?}",
                request_msg_res_3.err().unwrap()
            );
                continue;
            };

            let send_res_3 = client.send(request_msg_3).await;

            if let Err(err) = send_res_3 {
                error!("Unable to send Request UMessage: {:?}", err);
                continue;
            }

            tokio::time::sleep(Duration::from_millis(2000)).await;
        }
    }
}