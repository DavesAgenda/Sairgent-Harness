fn main() {
    let stderr_str = "DEBUG TOOL EXECUTION: dispatch_swo_internal called with payload length 248\n{\"__sairgent_sidechannel\": \"dispatch_swo\", \"payload\": \"Felicity, please prepare a report on PaaS.\"}\n";
    let mut dispatch_swos = Vec::new();

    for line in stderr_str.lines() {
        if line.contains("\"__sairgent_sidechannel\":") {
            let parsed_result: Result<serde_json::Value, _> = serde_json::from_str(line);
            match parsed_result {
                Ok(parsed_sidechannel) => {
                    println!("Parsed successfully: {:?}", parsed_sidechannel);
                    if parsed_sidechannel["__sairgent_sidechannel"] == "dispatch_swo" {
                        if let Some(payload_str) = parsed_sidechannel["payload"].as_str() {
                            println!("Found payload: {}", payload_str);
                            dispatch_swos.push(payload_str.to_string());
                        } else {
                            println!("NO PAYLOAD FIELD");
                        }
                    } else {
                        println!("UNKNOWN SIDECHANNEL");
                    }
                }
                Err(e) => {
                    println!("FAiled to parse json: {}", e);
                }
            }
        } else {
            println!("Echo: {}", line);
        }
    }

    println!("Final dispatch swos: {:?}", dispatch_swos);
}
