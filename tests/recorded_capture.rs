use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn recorded_bmp_is_sampled_cropped_and_validated_offline() {
    let dir = std::env::temp_dir().join(format!(
        "rhk-bmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let bmp = dir.join("fixture.bmp");
    let mut bytes = Vec::new();
    bytes.extend(b"BM");
    bytes.extend(70u32.to_le_bytes());
    bytes.extend([0; 4]);
    bytes.extend(54u32.to_le_bytes());
    bytes.extend(40u32.to_le_bytes());
    bytes.extend(2i32.to_le_bytes());
    bytes.extend((-2i32).to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend([0; 24]);
    bytes.extend([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    fs::write(&bmp, &bytes).unwrap();
    let profile = json!({"schema_version":1,"target":{"window_class":"FixtureOnly","window_title":null},"controls":{"abort":112,"reset":113,"pause":114,"resume":115},"tick_ms":10,"rules":[]});
    let obs = |path: &std::path::Path, width| json!({"target_present":true,"focused":true,"pixels":[{"point":{"x":0,"y":0},"color":123}],"screenshot":{"width":width,"height":2,"bmp_path":path}});
    let profile_path = dir.join("profile.json");
    let observation_path = dir.join("observation.json");
    let crop = dir.join("crop.bmp");
    fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    fs::write(
        &observation_path,
        serde_json::to_vec(&obs(&bmp, 2)).unwrap(),
    )
    .unwrap();
    let bad = dir.join("bad.bmp");
    fs::write(&bad, b"BM").unwrap();
    let mut bottom = bytes.clone();
    bottom[22..26].copy_from_slice(&2i32.to_le_bytes());
    bottom[54..62].copy_from_slice(&bytes[62..70]);
    bottom[62..70].copy_from_slice(&bytes[54..62]);
    let bottom_path = dir.join("bottom.bmp");
    fs::write(&bottom_path, bottom).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rhk"))
        .args(["mock-rpc"])
        .arg(&profile_path)
        .arg(&observation_path)
        .env("RHK_DATA_DIR", dir.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"id":1,"op":"observe_state","points":[{"x":0,"y":0},{"x":1,"y":1}]}),
        json!({"id":2,"op":"screenshot","path":crop,"region":{"x":0,"y":0,"width":0.5,"height":1}}),
        json!({"id":3,"op":"observe","observation":obs(&bad,2)}),
        json!({"id":4,"op":"observe","observation":obs(&bmp,3)}),
        json!({"id":5,"op":"observe","observation":obs(&bmp,16777217)}),
        json!({"id":6,"op":"observe","observation":obs(&bottom_path,2)}),
        json!({"id":7,"op":"observe_state","points":[{"x":1,"y":1}]}),
        json!({"id":8,"op":"observe","observation":{"target_present":true,"focused":true,"screenshot":{"width":2,"height":2,"bmp_path":bmp,"bgra":[]}}}),
        json!({"id":9,"op":"trace"}),
    ];
    for req in requests {
        writeln!(child.stdin.as_mut().unwrap(), "{}", req).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies[0]["result"]["pixels"][0]["color"], 123);
    assert_eq!(replies[0]["result"]["pixels"][1]["color"], 0x0f0e0d);
    assert_eq!(&fs::read(crop).unwrap()[54..], &[1, 2, 3, 4, 9, 10, 11, 12]);
    for i in [2, 3, 4, 7] {
        assert_eq!(replies[i]["ok"], false);
    }
    assert_eq!(replies[5]["ok"], true);
    assert_eq!(replies[6]["result"]["pixels"][0]["color"], 0x0f0e0d);
    assert_eq!(replies[8]["result"]["events"], json!([]));
    fs::remove_dir_all(dir).unwrap();
}
