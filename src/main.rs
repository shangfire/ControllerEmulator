/*
 * @Author: shanghanjin
 * @Date: 2025-01-22 17:32:42
 * @LastEditTime: 2025-01-22 17:36:14
 * @FilePath: \ControllerEmulator\src\main.rs
 * @Description: 
 */
use mlua::prelude::*;

fn main() -> LuaResult<()> {
    // 创建 Lua 实例
    let lua = Lua::new();

    // 执行 Lua 代码
    lua.load(r#"
        function add(a, b)
            return a + b
        end
    "#).exec()?;

    // 调用 Lua 函数
    let add: LuaFunction = lua.globals().get("add")?;
    let result: i32 = add.call((3, 5))?;
    println!("Result: {}", result); // 输出: Result: 8

    Ok(())
}