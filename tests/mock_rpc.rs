use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn mock_rpc_runs_engine_without_a_live_target() {
    let root = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join("RustyHotkey").join("tests").join(format!(
        "mock-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let profile = json!({"schema_version":1,"target":{"window_class":"FixtureOnly","window_title":null},"controls":{"abort":112,"reset":113,"pause":114,"resume":115},"tick_ms":10,"rules":[]});
    let observation = json!({"target_present":true,"focused":true,"keys_down":[],"pixels":[{"point":{"x":0.5,"y":0.5},"color":123}],"clipboard":"ready","ocr":[],"cursor":{"x":0.5,"y":0.5},"screenshot":{"width":2,"height":2,"solid_color":null,"bgra":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]}});
    let profile_path = dir.join("profile.json");
    let observation_path = dir.join("observation.json");
    let widget_path = dir.join("widget.lua");
    let crop_path = dir.join("crop.bmp");
    let debug_path = dir.join("debug.bmp");
    let debug_widget = dir.join("debug.lua");
    fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    fs::write(&observation_path, serde_json::to_vec(&observation).unwrap()).unwrap();
    fs::write(
        &widget_path,
        "return function(s) return {status=s.state.label} end",
    )
    .unwrap();
    fs::write(&debug_widget, "return function(s) return {elements={{type='box',region={x=0.1,y=0.1,width=0.5,height=0.5},color=16711680,fill_color=1382169}}} end").unwrap();
    let requests = [
        json!({"id":1,"op":"arm"}),
        json!({"id":2,"op":"execute","actions":[{"type":"key","key":65,"down":true}]}),
        json!({"id":3,"op":"pause"}),
        json!({"id":4,"op":"advance","ms":25}),
        json!({"id":5,"op":"trace"}),
        json!({"id":6,"op":"screenshot","path":crop_path,"region":{"x":0,"y":0,"width":0.5,"height":1}}),
        json!({"id":7,"op":"bad"}),
        json!({"id":8,"op":"observe","observation":{"target_present":true,"focused":true,"cursor":{"x":2,"y":0}}}),
        json!({"id":9,"op":"status"}),
        json!({"id":10,"op":"widgets","paths":[widget_path],"state":{"label":"Fixture"}}),
        json!({"id":11,"op":"widget_state","state":{"label":"Changed"}}),
        json!({"id":12,"op":"screenshot"}),
        json!({"id":13,"op":"geometry"}),
        json!({"id":14,"op":"observe_state","points":[{"x":0.5,"y":0.5}]}),
        json!({"id":15,"op":"observe_state","points":[{"x":2,"y":0}]}),
        json!({"id":16,"op":"observe","observation":{"target_present":true,"focused":false,"screenshot":{"width":2,"height":2,"solid_color":0}}}),
        json!({"id":17,"op":"observe_state","points":[{"x":0.5,"y":0.5}]}),
        json!({"id":18,"op":"observe","observation":{"target_present":true,"focused":true,"screenshot":{"width":64,"height":64,"solid_color":0}}}),
        json!({"id":19,"op":"widgets","paths":[debug_widget],"state":{}}),
        json!({"id":20,"op":"debug_screenshot","path":debug_path}),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_rhk"))
        .arg("mock-rpc")
        .arg(&profile_path)
        .arg(&observation_path)
        .env("RHK_DATA_DIR", dir.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "invalid-json").unwrap();
        for request in requests {
            writeln!(stdin, "{}", request).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 21);
    assert_eq!(lines[0]["ok"], false);
    let reply = |id: u64| lines.iter().find(|line| line["id"] == id).unwrap();
    assert_eq!(reply(1)["ok"], true);
    assert_eq!(reply(4)["result"]["now_ms"], 25);
    let trace = reply(5)["result"]["events"].as_array().unwrap();
    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0]["event"]["down"], true);
    assert_eq!(trace[1]["event"]["down"], false);
    assert_eq!(reply(6)["result"]["width"], 1);
    assert_eq!(reply(6)["result"]["height"], 2);
    let bitmap = fs::read(&crop_path).unwrap();
    assert_eq!(&bitmap[54..], &[1, 2, 3, 4, 9, 10, 11, 12]);
    assert_eq!(reply(7)["ok"], false);
    assert_eq!(reply(8)["ok"], false);
    assert_eq!(reply(9)["result"]["present"], true);
    assert_eq!(reply(10)["result"]["spec"]["status"], "Fixture");
    assert_eq!(reply(11)["result"]["spec"]["status"], "Changed");
    let capture = std::path::Path::new(reply(12)["result"]["path"].as_str().unwrap());
    assert!(capture.starts_with(dir.join("state")));
    assert!(
        !capture.exists(),
        "default mock captures must be cleaned up"
    );
    assert_eq!(reply(12)["result"]["simulated"], true);
    assert_eq!(
        reply(13)["result"],
        json!({"width":2,"height":2,"screen_x":0,"screen_y":0,"simulated":true})
    );
    assert_eq!(reply(14)["result"]["pixels"][0]["color"], 123);
    assert_eq!(reply(14)["result"]["geometry"]["width"], 2);
    assert_eq!(reply(15)["ok"], false);
    assert_eq!(reply(17)["result"]["pixels"], json!([]));
    assert_eq!(reply(17)["result"]["status"]["focused"], false);
    #[cfg(windows)]
    {
        assert_eq!(reply(20)["ok"], true);
        assert_eq!(reply(20)["result"]["width"], 64);
        let image = fs::read(&debug_path).unwrap();
        assert_eq!(&image[..2], b"BM");
        assert!(image[54..].chunks_exact(4).any(|p| p[..3] == [0, 0, 255]));
        assert_eq!(reply(20)["result"]["simulated"], true);
    }
    #[cfg(not(windows))]
    assert_eq!(reply(20)["ok"], false);
    fs::remove_dir_all(dir).unwrap();
}
