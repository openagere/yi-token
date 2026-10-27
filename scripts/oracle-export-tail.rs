
fn proto_name(p: Protocol) -> &'static str {
    match p {
        Protocol::Responses => "responses",
        Protocol::OpenaiChat => "openai_chat",
        Protocol::Anthropic => "anthropic",
    }
}

/// (accepted, upstream body) for one case.
fn run(client: Protocol, upstream: Protocol, body: &Value) -> (bool, Option<Value>) {
    if client::validate_client_options(client, upstream, body).is_err() {
        return (false, None);
    }
    let Ok(normalized) = client::normalize_body(client, body) else {
        return (false, None);
    };
    if client::validate_for_upstream(upstream, &normalized).is_err() {
        return (false, None);
    }
    if client == upstream {
        let mut prepared = body.clone();
        let mut prov = provider(upstream);
        if prepare::prepare_request(&mut prepared, client, &mut prov, &Capabilities::default())
            .is_err()
        {
            return (false, None);
        }
        if let Some(obj) = prepared.as_object_mut() {
            obj.insert("model".into(), json!("up"));
        }
        return (true, Some(prepared));
    }
    if upstream == Protocol::Responses {
        // Old pipeline forwards the normalized Responses body as-is.
        if serde_json::from_value::<ResponsesRequest>(normalized.clone()).is_err() {
            return (false, None);
        }
        let mut n = normalized;
        n["model"] = json!("up");
        return (true, Some(n));
    }
    let Ok(request) = serde_json::from_value::<ResponsesRequest>(normalized) else {
        return (false, None);
    };
    let prov = provider(upstream);
    let built = match upstream {
        Protocol::Anthropic => anthropic::build_request(&request, &prov, "up")
            .map(|r| serde_json::to_value(r).unwrap()),
        Protocol::OpenaiChat => chat_completions::build_request(&request, &prov, "up")
            .map(|r| serde_json::to_value(r).unwrap()),
        Protocol::Responses => unreachable!(),
    };
    match built {
        Ok(v) => (true, Some(v)),
        Err(_) => (false, None),
    }
}

#[test]
fn export_corpus() {
    let mut out = String::new();
    for (label, client_protocol, body) in corpus() {
        let rest = label.split_once('/').unwrap().1;
        let (field, value) = match rest.split_once('=') {
            Some((f, v)) => (f.to_string(), v.to_string()),
            None => (rest.to_string(), String::new()),
        };
        for upstream in PROTOCOLS {
            let (ok, built) = run(client_protocol, upstream, &body);
            let mut rec = json!({
                "case_id": format!("{label} -> {}", proto_name(upstream)),
                "inbound": proto_name(client_protocol),
                "upstream": proto_name(upstream),
                "field": field,
                "value": value,
                "request": body,
                "outcome": if ok { "accepted" } else { "rejected" },
            });
            if let Some(b) = built {
                rec["upstream_body"] = b;
            }
            out.push_str(&rec.to_string());
            out.push('\n');
        }
    }
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/target/request_pipeline.jsonl"),
        out,
    )
    .unwrap();
}
