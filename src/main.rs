/*
 * @Author: shanghanjin
 * @Date: 2025-01-22 17:32:42
 * @LastEditTime: 2025-02-20 14:42:38
 * @FilePath: \ControllerEmulator\src\main.rs
 * @Description: 
 */
use mlua::prelude::*;
use std::fs;
use std::io;

fn main() -> LuaResult<()> {
    // 创建 Lua 实例
    let lua = Lua::new();
    let scripts_dir = "scripts";

    // 读取目录下的所有.lua文件
    let mut scripts = vec![];
    if let Ok(entries) = fs::read_dir(scripts_dir) {
        for (id, entry) in entries.filter_map(Result::ok).enumerate() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("lua") {
                scripts.push((id, path.clone()));
            }
        }
    }

    if scripts.is_empty() {
        println!("can't find any lua script in {}", scripts_dir);
        return Ok(());
    }

    // 展示可用的lua脚本
    println!("usable scripts:");
    for (id, path) in &scripts {
        println!("[{}]: {}", id, path.display());
    }

    // 读取用户输入的脚本编号
    let mut input = String::new();
    println!("please select a script to run: ");
    io::stdin().read_line(&mut input).expect("read line failed");
    let input = input.trim();
    let selected_id:usize = match input.parse() {
        Ok(id) if id < scripts.len() => id,
        _ => {
            println!("invalid script id: {}", input);
            return Ok(());
        }
    };

    // 读取并执行lua脚本
    let script_path = &scripts[selected_id].1;
    let script_content = fs::read_to_string(script_path).expect("读取脚本失败");

    lua.load(&script_content).exec()?;
    println!("executed script: {}", script_path.display()); // 输出: Result: 8

    Ok(())
}
