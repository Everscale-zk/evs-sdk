extern crate ton_sdk;
extern crate hex;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;

use rand::{thread_rng, Rng};
use ton_block::{Message, MsgAddressExt, MsgAddressInt, InternalMessageHeader, Grams, 
    ExternalInboundMessageHeader, CurrencyCollection, Serializable};
use tvm::bitstring::Bitstring;
use tvm::types::AccountId;
use ed25519_dalek::Keypair;
use futures::Stream;
use sha2::Sha512;

use abi_lib_dynamic::json_abi::decode_function_responce;

use ton_sdk::*;

const STD_CONFIG: &str = r#"
{
    "db_config": {
        "servers": ["142.93.137.28:28015"],
        "db_name": "blockchain"
    },
    "kafka_config": {
        "servers": ["142.93.137.28:9092"],
        "topic": "requests-1",
        "ack_timeout": 1000
    }
}"#;


const WALLET_ABI: &str = r#"{
	"ABI version" : 0,

	"functions" :	[
	    {
	        "inputs": [
	            {
	                "name": "recipient",
	                "type": "bits256"
	            },
	            {
	                "name": "value",
	                "type": "duint"
	            }
	        ],
	        "name": "sendTransaction",
					"signed": true,
	        "outputs": [
	            {
	                "name": "transaction",
	                "type": "uint64"
	            },
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
	    {
	        "inputs": [
						  {
	                "name": "type",
	                "type": "uint8"
	            },
							{
	                "name": "value",
	                "type": "duint"
	            },
							{
	                "name": "meta",
	                "type": "bitstring"
	            }
					],
	        "name": "createLimit",
					"signed": true,
	        "outputs": [
							{
	                "name": "limitId",
	                "type": "uint8"
	            },
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
	    {
	        "inputs": [
							{
	                "name": "limitId",
	                "type": "uint8"
	            },
							{
	                "name": "value",
	                "type": "duint"
	            },
							{
	                "name": "meta",
	                "type": "bitstring"
	            }
	        ],
	        "name": "changeLimitById",
					"signed": true,
	        "outputs": [
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
			{
	        "inputs": [
							{
	                "name": "limitId",
	                "type": "uint8"
	            }
	        ],
	        "name": "removeLimit",
					"signed": true,
	        "outputs": [
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
			{
	        "inputs": [
							{
	                "name": "limitId",
	                "type": "uint8"
	            }
	        ],
	        "name": "getLimitById",
	        "outputs": [
							{
									"name": "limitInfo",
					        "type": "tuple",
					        "components": [
											{
					                "name": "value",
					                "type": "duint"
					            },
											{
					                "name": "type",
					                "type": "uint8"
					            },
											{
					                "name": "meta",
					                "type": "bitstring"
					            }
									]
							},
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
			{
	        "inputs": [],
	        "name": "getLimits",
	        "outputs": [
							{
									"name": "list",
					        "type": "uint8[]"
							},
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
			{
	        "inputs": [],
	        "name": "getVersion",
	        "outputs": [
							{
									"name": "version",
					        "type": "tuple",
					        "components": [
											{
					                "name": "major",
					                "type": "uint16"
					            },
											{
					                "name": "minor",
					                "type": "uint16"
					            }
									]
							},
							{
	                "name": "error",
	                "type": "int8"
	            }
	        ]
	    },
			{
	        "inputs": [],
	        "name": "getBalance",
	        "outputs": [
							{
	                "name": "balance",
	                "type": "uint64"
	            }
	        ]
	    },
			{
	        "inputs": [],
	        "name": "constructor",
	        "outputs": []							
	    },
			{
	        "inputs": [{"name": "address", "type": "bits256" }],
	        "name": "setSubscriptionAccount",
					"signed": true,
	        "outputs": []							
	    },
			{
	        "inputs": [],
	        "name": "getSubscriptionAccount",
	        "outputs": [{"name": "address", "type": "bits256" }]							
	    }
	]
}
"#;

// Create message "from wallet" to transfer some funds 
// from one account to another
pub fn create_external_transfer_funds_message(src: AccountId, dst: AccountId, value: u128) -> Message {
    
    let mut rng = thread_rng();    
    let mut msg = Message::with_ext_in_header(
        ExternalInboundMessageHeader {
            src: MsgAddressExt::with_extern(&Bitstring::from(rng.gen::<u64>())).unwrap(),
            dst: MsgAddressInt::with_standart(None, 0, src.clone()).unwrap(),
            import_fee: Grams::default(),
        }
    );

    let mut balance = CurrencyCollection::default();
    balance.grams = Grams(value.into());

    let int_msg_hdr = InternalMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, 0, src).unwrap(),
            MsgAddressInt::with_standart(None, 0, dst).unwrap(),
            balance);

    msg.body = Some(int_msg_hdr.write_to_new_cell().unwrap().into());

    msg
}

fn deploy_contract_and_wait(code_file_name: &str, abi: &str, constructor_params: &str, key_pair: &Keypair) -> AccountId {
    // read image from file and construct ContractImage
    let mut state_init = std::fs::File::open(code_file_name).expect("Unable to open contract code file");

    let contract_image = ContractImage::from_state_init_and_key(&mut state_init, &key_pair).expect("Unable to parse contract code file");

    let account_id = contract_image.account_id();

    // before deploying contract need to transfer some funds to its address
    //println!("Account ID to take some grams {}\n", account_id);
    let msg = create_external_transfer_funds_message(AccountId::from([0_u8; 32]), account_id.clone(), 100);
    Contract::send_message(msg).unwrap();


    // call deploy method
    let changes_stream = Contract::deploy_json("constructor".to_owned(), constructor_params.to_owned(), abi.to_owned(), contract_image, Some(key_pair))
        .expect("Error deploying contract");

    // wait transaction id in message-status or 
    // wait message will done and find transaction with the message

    // wait transaction id in message-status 
    let mut tr_id = None;
    for state in changes_stream.wait() {
        if let Err(e) = state {
            panic!("error next state getting: {}", e);
        }
        if let Ok(s) = state {
            //println!("next state: {:?}", s);
            if s.message_state == MessageState::Finalized {
                tr_id = Some(s.message_id.clone());
                break;
            }
        }
    }
    // contract constructor doesn't return any values so there are no output messages in transaction
    // so just check deployment transaction created
    let _tr_id = tr_id.expect("Error: no transaction id");

	account_id
}


fn call_contract_and_wait(address: AccountId, func: &str, input: &str, abi: &str, key_pair: &Keypair) -> String {

    let contract = Contract::load(address)
        .expect("Error calling load Contract")
        .wait()
        .next()
        .expect("Error unwrap stream next while loading Contract")
        .expect("Error unwrap result while loading Contract");

    // call needed method
    let changes_stream = contract.call_json(func.to_owned(), input.to_owned(), abi.to_owned(), Some(&key_pair))
        .expect("Error calling contract method");

    // wait transaction id in message-status 
    let mut tr_id = None;
    for state in changes_stream.wait() {
        if let Err(e) = state {
            panic!("error next state getting: {}", e);
        }
        if let Ok(s) = state {
            //println!("next state: {:?}", s);
            if s.message_state == MessageState::Finalized {
                tr_id = Some(s.message_id.clone());
                break;
            }
        }
    }
    let tr_id = tr_id.expect("Error: no transaction id");

    // OR 
    // wait message will done and find transaction with the message

    // load transaction object
    let tr = Transaction::load(tr_id)
        .expect("Error calling load Transaction")
        .wait()
        .next()
        .expect("Error unwrap stream next while loading Transaction")
        .expect("Error unwrap result while loading Transaction");

    // take external outbound message from the transaction
    let out_msg = tr.load_out_messages()
        .expect("Error calling load out messages")
        .wait()
        .find(|msg| msg.as_ref().expect("erro unwrap out message").msg_type() == MessageType::ExternalOutbound)
            .expect("erro unwrap out message 2")
            .expect("erro unwrap out message 3");

    // take body from the message
    let responce = out_msg.body().into();

    // decode the body by ABI
    let result = decode_function_responce(abi.to_owned(), func.to_owned(), responce)
        .expect("Error decoding result");

    println!("Contract call result: {}\n", result);

	result

    // this way it is need:
    // 1. message status with transaction id or transaction object with in-message id
    // 2. transaction object with out messages ids
    // 3. message object with body
}

fn call_create(current_address: &mut Option<AccountId>) {
	println!("Creating new wallet account");

    // generate key pair
    let mut csprng = rand::rngs::OsRng::new().unwrap();
    let keypair = Keypair::generate::<Sha512, _>(&mut csprng);
   
	// deploy wallet
    let wallet_address = deploy_contract_and_wait("Wallet.tvc", WALLET_ABI, "{}", &keypair);
	let str_address = hex::encode(wallet_address.as_slice());

    println!("Acoount created. Address {}", str_address);


	std::fs::write("last", wallet_address.as_slice()).expect("Couldn't save wallet address");
	std::fs::write(str_address, &keypair.to_bytes().to_vec()).expect("Couldn't save wallet key pair");

	*current_address = Some(wallet_address);
}

fn call_get_balance(current_address: &Option<AccountId>, params: &[&str]) {
	let address = if params.len() > 0 {
		AccountId::from(hex::decode(params[0]).unwrap())
	} else {
		if let Some(addr) = current_address.clone() {
			addr
		} else {
			println!("Current address not set");
			return;
		}
	};

	let contract = Contract::load(address)
        .expect("Error calling load Contract")
        .wait()
        .next()
        .expect("Error unwrap stream next while loading Contract")
        .expect("Error unwrap result while loading Contract");

	let balance = contract.balance_grams();

	println!("Account balance {}", balance);
}

#[derive(Deserialize)]
struct SendTransactionAnswer {
	transaction: String,
	error: String
}

fn call_send_transaction(current_address: &Option<AccountId>, params: &[&str]) {
    if params.len() < 2 {
        println!("Not enough parameters");
        return;
    }

	let address = if let Some(addr) = current_address {
		addr.clone()
	} else {
		println!("Current address not set");
		return;
	};

    let str_params = format!("{{ \"recipient\" : \"x{}\", \"value\": \"{}\" }}", params[0], params[1]);

	let pair = std::fs::read(hex::encode(address.as_slice())).expect("Couldn't read key pair");
	let pair = Keypair::from_bytes(&pair).expect("Couldn't restore key pair");

	let answer = call_contract_and_wait(address, "sendTransaction", &str_params, WALLET_ABI, &pair);


    let answer: SendTransactionAnswer = serde_json::from_str(&answer).unwrap();

	let transaction = u64::from_str_radix(&answer.transaction[2..], 16).expect("Couldn't parse transaction number");

    println!("Transaction ID {}", transaction);
}

const HELP: &str = r#"
Supported commands:
    balance <address>       - get the account balance. If address is not provided current address is used
    create                  - create new wallet account and set as current
    send <address> <value>  - send <value> grams to <address>
    exit                    - exit program"#;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let config = if args.len() > 3 && args[2] == "-config" {
        std::fs::read_to_string(&args[3]).expect("Couldn't read config file")
    } else {
        STD_CONFIG.to_owned()
    };

    init_json(config).expect("Couldn't establish connection");
    println!("Connection established");

	let mut current_address: Option<AccountId> = None;

	if let Ok(address) = std::fs::read("last_address") {
		current_address = Some(AccountId::from(address));

		println!("Wallet address {}", hex::encode(current_address.clone().unwrap().as_slice()));
	} else {
		println!("Wallet address not assigned. Create new wallet");
	}

    println!("Enter command");

    loop {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).expect("error: unable to read user input");

        let params: Vec<&str> = input.split_whitespace().collect();

        if params.len() == 0 {
        	continue;
        }

        match params[0].as_ref() {
        	"help" => println!("{}", HELP),
			"balance" => call_get_balance(&current_address, &params[1..]),
            "create" => call_create(&mut current_address),
            "send" => call_send_transaction(&current_address, &params[1..]),
            "exit" => break,
            _ => println!("Unknown command")
        }
    }
}
