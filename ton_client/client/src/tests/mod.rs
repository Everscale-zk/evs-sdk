/*
* Copyright 2018-2020 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and
* limitations under the License.
*/

use crate::{
    tc_create_context, tc_destroy_context, JsonResponse,
    error::{ApiError, ApiResult},
    crypto::keys::KeyPair,
    queries::{ParamsOfWaitForCollection, ResultOfWaitForCollection},
    contracts::{
        EncodedMessage,
        deploy::{ParamsOfDeploy, ResultOfDeploy},
        run::{ParamsOfRun, ResultOfRun, RunFunctionCallSet},
    },
    client::{ResultOfCreateContext, ParamsOfUnregisterCallback}
};
use super::InteropContext;
use super::{tc_json_request, tc_json_request_async, InteropString};
use super::{tc_read_json_response, tc_destroy_json_response};
use serde_json::Value;
use serde::Serialize;
use serde::de::DeserializeOwned;
use rand::Rng;
use std::collections::HashMap;
use std::sync::{
    Mutex, 
    mpsc::{channel, Sender}};

mod common;

const ROOT_CONTRACTS_PATH: &str = "src/tests/contracts/";
const LOG_CGF_PATH: &str = "src/tests/log_cfg.yaml";

lazy_static::lazy_static! {
    static ref GIVER_ADDRESS: &'static str = "0:841288ed3b55d9cdafa806807f02a0ae0c169aa5edfe88a789a6482429756a94";
    static ref WALLET_ADDRESS: &'static str = "0:2bb4a0e8391e7ea8877f4825064924bd41ce110fce97e939d3323999e1efbb13";
	static ref WALLET_KEYS: Option<KeyPair> = get_wallet_keys();

	static ref ABI_VERSION: u8 = u8::from_str_radix(&std::env::var("ABI_VERSION").unwrap_or("2".to_owned()), 10).unwrap();
	static ref CONTRACTS_PATH: String = format!("{}abi_v{}/", ROOT_CONTRACTS_PATH, *ABI_VERSION);
	static ref NODE_ADDRESS: String = std::env::var("TON_NETWORK_ADDRESS")
		//.unwrap_or("cinet.tonlabs.io".to_owned());
		.unwrap_or("http://localhost".to_owned());
		//.unwrap_or("net.ton.dev".to_owned());
	static ref NODE_SE: bool = std::env::var("USE_NODE_SE").unwrap_or("true".to_owned()) == "true".to_owned();

	pub static ref SUBSCRIBE_ABI: Value = read_abi(CONTRACTS_PATH.clone() + "Subscription.abi.json");
	pub static ref PIGGY_BANK_ABI: Value = read_abi(CONTRACTS_PATH.clone() + "Piggy.abi.json");
    pub static ref WALLET_ABI: Value = read_abi(CONTRACTS_PATH.clone() + "LimitWallet.abi.json");
    pub static ref SIMPLE_WALLET_ABI: Value = read_abi(CONTRACTS_PATH.clone() + "Wallet.abi.json");
	pub static ref GIVER_ABI: Value = read_abi(ROOT_CONTRACTS_PATH.to_owned() + "Giver.abi.json");
	pub static ref GIVER_WALLET_ABI: Value = read_abi(ROOT_CONTRACTS_PATH.to_owned() + "GiverWallet.abi.json");
	pub static ref HELLO_ABI: Value = read_abi(CONTRACTS_PATH.clone() + "Hello.abi.json");

    pub static ref SUBSCRIBE_IMAGE: Vec<u8> = std::fs::read(CONTRACTS_PATH.clone() + "Subscription.tvc").unwrap();
	pub static ref PIGGY_BANK_IMAGE: Vec<u8> = std::fs::read(CONTRACTS_PATH.clone() + "Piggy.tvc").unwrap();
	pub static ref WALLET_IMAGE: Vec<u8> = std::fs::read(CONTRACTS_PATH.clone() + "LimitWallet.tvc").unwrap();
	pub static ref SIMPLE_WALLET_IMAGE: Vec<u8> = std::fs::read(CONTRACTS_PATH.clone() + "Wallet.tvc").unwrap();
    pub static ref HELLO_IMAGE: Vec<u8> = std::fs::read(CONTRACTS_PATH.clone() + "Hello.tvc").unwrap();

    pub static ref REQUESTS: Mutex<HashMap<u32, Sender<JsonResponse>>> = Mutex::new(HashMap::new());
    pub static ref CALLBACKS: Mutex<HashMap<u32, Box<dyn Fn(u32, String, String, u32) + Send>>> = Mutex::new(HashMap::new());
}

fn read_abi(path: String) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path).unwrap()
    ).unwrap()
}

fn get_wallet_keys() -> Option<KeyPair> {
    if *NODE_SE {
        return None;
    }

    let mut keys_file = dirs::home_dir().unwrap();
    keys_file.push("giverKeys.json");
    let keys = std::fs::read_to_string(keys_file).unwrap();

    Some(serde_json::from_str(&keys).unwrap())
}

struct SimpleLogger;

const MAX_LEVEL: log::LevelFilter = log::LevelFilter::Warn;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() < MAX_LEVEL
    }

    fn log(&self, record: &log::Record) {
        match record.level() {
            log::Level::Error | log::Level::Warn => {
                eprintln!("{}", record.args());
            }
            _ => {
                println!("{}", record.args());
            }
        }
    }

    fn flush(&self) {}
}

#[derive(Clone)]
pub(crate) struct TestClient {
    context: InteropContext,
}

extern "C" fn on_result(request_id: u32, result_json: InteropString, error_json: InteropString, flags: u32) {
    TestClient::on_result(request_id, result_json, error_json, flags)
}

extern "C" fn on_callback(request_id: u32, result_json: InteropString, error_json: InteropString, flags: u32) {
    TestClient::callback(request_id, result_json, error_json, flags)
}

impl TestClient {
    pub(crate) fn init_log() {
        let log_cfg_path = LOG_CGF_PATH;
        let _ = log4rs::init_file(log_cfg_path, Default::default());
    }

    pub(crate) fn get_network_address() -> String {
        NODE_ADDRESS.clone()
    }

    pub(crate) fn new() -> Self {
        Self::new_with_config(json!({
            "network": {
                "server_address": Self::get_network_address()
            }
        }))
    }

    pub(crate) fn new_with_config(config: Value) -> Self {
        let _ = log::set_boxed_logger(Box::new(SimpleLogger))
            .map(|()| log::set_max_level(MAX_LEVEL));

        let response = unsafe {
            let response_ptr = tc_create_context(InteropString::from(&config.to_string()));
            let interop_response = tc_read_json_response(response_ptr);
            let response = interop_response.to_response();
            tc_destroy_json_response(response_ptr);
            response
        };

        let context = if response.error_json.is_empty() {
            let result: ResultOfCreateContext = serde_json::from_str(&response.result_json).unwrap();
            result.handle
        } else {
            panic!("tc_create_context returned error: {}", response.error_json);
        };

        let client = Self { context };
        client
    }

    pub(crate) fn request_json(&self, method: &str, params: Value) -> ApiResult<Value> {
        let response = unsafe {
            let params_json = if params.is_null() { String::new() } else { params.to_string() };
            let response_ptr = tc_json_request(
                self.context,
                InteropString::from(&method.to_string()),
                InteropString::from(&params_json),
            );
            let interop_response = tc_read_json_response(response_ptr);
            let response = interop_response.to_response();
            tc_destroy_json_response(response_ptr);
            response
        };
        if response.error_json.is_empty() {
            if response.result_json.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(serde_json::from_str(&response.result_json).unwrap())
            }
        } else {
            Err(serde_json::from_str(&response.error_json).unwrap())
        }
    }

    pub(crate) fn request<P, R>(&self, method: &str, params: P) -> R
        where P: Serialize, R: DeserializeOwned {
        let params = serde_json::to_value(params)
            .map_err(|err| ApiError::invalid_params("", err)).unwrap();
        let result = self.request_json(method, params).unwrap();
        serde_json::from_value(result)
            .map_err(|err| ApiError::invalid_params("", err))
            .unwrap()
    }

    fn on_result(request_id: u32, result_json: InteropString, error_json: InteropString, _flags: u32) {
        let response = JsonResponse {
            result_json: result_json.to_string(),
            error_json: error_json.to_string()
        };

        REQUESTS.lock().unwrap()
            .remove(&request_id)
            .unwrap()
            .send(response)
            .unwrap()
    }

    pub(crate) fn request_json_async(&self, method: &str, params: Value) -> ApiResult<Value> {
        let request_id = rand::thread_rng().gen::<u32>();
        let (sender, receiver) = channel();
        REQUESTS.lock().unwrap().insert(request_id, sender);

        unsafe {
            let params_json = if params.is_null() { String::new() } else { params.to_string() };
            tc_json_request_async(
                self.context,
                InteropString::from(&method.to_string()),
                InteropString::from(&params_json),
                request_id,
                on_result
            );
        };

        let response = receiver.recv().unwrap();
        if response.error_json.is_empty() {
            if response.result_json.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(serde_json::from_str(&response.result_json).unwrap())
            }
        } else {
            Err(serde_json::from_str(&response.error_json).unwrap())
        }
    }

    pub(crate) fn request_async<P, R>(&self, method: &str, params: P) -> R
        where P: Serialize, R: DeserializeOwned {
        let params = serde_json::to_value(params)
            .map_err(|err| ApiError::invalid_params("", err)).unwrap();
        let result = self.request_json_async(method, params).unwrap();
        serde_json::from_value(result)
            .map_err(|err| ApiError::invalid_params("", err))
            .unwrap()
    }

    fn callback(request_id: u32, result_json: InteropString, error_json: InteropString, flags: u32) {
        let callbacks_lock = CALLBACKS.lock().unwrap();
        let callback = callbacks_lock.get(&request_id).unwrap();
        callback(request_id, result_json.to_string(), error_json.to_string(), flags);
    }

    pub(crate) fn register_callback<R: DeserializeOwned>(
        &self,
        callback_id: Option<u32>,
        callback: impl Fn(u32, ApiResult<R>, u32) + Send + Sync + 'static
    ) -> u32 {
        let callback = move |request_id: u32, result_json: String, error_json: String, flags: u32| {
            let result = if !result_json.is_empty() {
                Ok(serde_json::from_str(&result_json).unwrap())
            } else {
                Err(serde_json::from_str(&error_json).unwrap())
            };
            callback(request_id, result, flags)
        };
        let callback_id = callback_id.unwrap_or_else(|| rand::thread_rng().gen::<u32>());
        CALLBACKS.lock().unwrap().insert(callback_id, Box::new(callback));
        unsafe {
            tc_json_request_async(
                self.context,
                InteropString::from("client.register_callback"),
                InteropString::from(""),
                callback_id,
                on_callback
            );
        };

        callback_id
    }

    pub(crate) fn unregister_callback(&self, callback_id: u32) {
        let _: () = self.request(
            "client.unregister_callback",
            ParamsOfUnregisterCallback {
                callback_id
            }
        );

        CALLBACKS.lock().unwrap().remove(&callback_id);
    }

    pub(crate) fn get_grams_from_giver(&self, account: &str, value: Option<u64>) {
        let run_result: ResultOfRun = if *NODE_SE {
            self.request(
                "contracts.run",
                ParamsOfRun {
                    address: GIVER_ADDRESS.to_owned(),
                    call_set: RunFunctionCallSet {
                        abi: GIVER_ABI.clone(),
                        function_name: "sendGrams".to_owned(),
                        header: None,
                        input: json!({
                            "dest": account,
                            "amount": value.unwrap_or(500_000_000u64)
                        }),
                    },
                    key_pair: None,
                    try_index: None,
                },
            )
        } else {
            self.request(
                "contracts.run",
                ParamsOfRun {
                    address: WALLET_ADDRESS.to_owned(),
                    call_set: RunFunctionCallSet {
                        abi: GIVER_WALLET_ABI.clone(),
                        function_name: "sendTransaction".to_owned(),
                        header: None,
                        input: json!({
                            "dest": account.to_string(),
                            "value": value.unwrap_or(500_000_000u64),
                            "bounce": false
                        }),
                    },
                    key_pair: WALLET_KEYS.clone(),
                    try_index: None,
                },
            )
        };

        // wait for grams recieving
        for message in run_result.transaction["out_messages"].as_array().unwrap() {
            let message: ton_sdk::Message = serde_json::from_value(message.clone()).unwrap();
            if ton_sdk::MessageType::Internal == message.msg_type() {
                let _: ResultOfWaitForCollection = self.request(
                    "queries.wait_for_collection",
                    ParamsOfWaitForCollection {
                        collection: "transactions".to_owned(),
                        filter: Some(json!({
                            "in_msg": { "eq": message.id()}
                        })),
                        result: "id".to_owned(),
                        timeout: Some(ton_sdk::types::DEFAULT_WAIT_TIMEOUT),
                    },
                );
            }
        }
    }

    pub(crate) fn deploy_with_giver(&self, params: ParamsOfDeploy, value: Option<u64>) -> String {
        let msg: EncodedMessage = self.request(
            "contracts.deploy.message",
            params.clone(),
        );

        self.get_grams_from_giver(&msg.address.unwrap(), value);

        let result: ResultOfDeploy = self.request(
            "contracts.deploy",
            params,
        );

        result.address
    }

    pub(crate) fn generate_kepair(&self) -> KeyPair {
        self.request("crypto.generate_random_sign_keys", ())
    }


    pub(crate) fn get_giver_address() -> String {
        if *NODE_SE {
            GIVER_ADDRESS.to_owned()
        } else {
            WALLET_ADDRESS.to_owned()
        }
    }
}

impl Drop for TestClient {
    fn drop(&mut self) {
        unsafe {
            if self.context != 0 {
                tc_destroy_context(self.context)
            }
        }
    }
}
