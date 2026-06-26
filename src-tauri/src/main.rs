// Prompt Pocket 二进制入口
// 防止 Windows 上启动时额外弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    prompt_pocket_lib::run();
}
