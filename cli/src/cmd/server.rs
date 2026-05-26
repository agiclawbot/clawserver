//! `clawctl server` —— 启动 HTTP 服务。
use clap::Args as ClapArgs;

use super::Ctx;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// 自定义绑定地址
    #[arg(long, default_value = "0.0.0.0:3385")]
    pub bind: String,
}

pub async fn run(_ctx: &Ctx, args: Args) -> anyhow::Result<()> {
    eprintln!(
        "[TODO] clawctl server --bind {} (not implemented yet, use `cargo run -p clawserver`)",
        args.bind
    );
    Ok(())
}
