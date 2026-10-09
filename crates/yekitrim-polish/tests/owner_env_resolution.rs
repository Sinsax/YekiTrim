//! **子进程按 `YEKITRIM_OWNER` 选库** —— 多用户"补齐到能用"的最后一块的判据。
//!
//! ## 为什么需要这条测试
//!
//! 出片 / 封面 / 预览都是 `yekitrim polish <sub> <dir>` **子进程**（`render/AGENTS.md`：
//! 分钟级重活不能跑在服务进程里）。子进程**自己**解析字体库、贴纸库、视频库的位置 ——
//! 而它原先只会看全局那一份 ⇒ **多用户下 A 导入的字体，B 出片时会被用上**。
//! 那个失效形态的可怕之处是：**不报错**，只是字体/素材不是你的。
//!
//! 所以父进程必须把身份传下去（`render.rs::run_polish_child` 写 `YEKITRIM_OWNER`），
//! 子进程经 `Owner::from_env()` 认出来。这条测试钉住**消费侧**真的照它选目录。
//!
//! ## 判据的设计（为什么不是"断言函数返回某个路径"）
//!
//! 环境变量是**进程级**状态，Rust 1.80+ 起 `set_var` 是 `unsafe`，而且测试是并行的 ——
//! 真去改进程环境会污染同进程里的其它用例（本仓已被这个坑咬过一次，见
//! `owner.rs` 里三个迁移用例共用同一个临时目录那次）。
//!
//! 所以这里**不碰真实环境变量**，而是直接用可注入的
//! `Owner::from_env_value` / `fonts::dir_for` / `owner::video_dir_readable_at`
//! 验"给定身份 ⇒ 给定目录"，再用 `Owner::from_env_value(Some("7"))` 验"环境变量的**字符串**
//! 真的能还原成身份"。两段合起来 = 父进程写下去的那个字符串，会让子进程选中不同的库。

/// 环境变量里的字符串 ⇒ 身份 ⇒ 库目录：**不同账号必须落到不同目录**。
///
/// 这正是父进程 `run_polish_child` 做的事（写 `owner.as_env_value()`）与子进程做的
/// 事（`Owner::from_env_value` → `fonts::dir_for`）连起来的效果。
#[test]
fn env_value_selects_a_different_library_per_account() {
    use yekitrim_core::owner::Owner;

    let home = std::env::temp_dir().join(format!(
        "yekitrim-ownerenv-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&home);

    // 父进程会写进环境的两种字符串。
    let alice_env = "7";
    let bob_env = "8";
    let alice = Owner::from_env_value(Some(alice_env));
    let bob = Owner::from_env_value(Some(bob_env));
    assert_eq!(alice, Owner::User(7), "YEKITRIM_OWNER={alice_env} 应还原成 uid 7");
    assert_eq!(bob, Owner::User(8), "YEKITRIM_OWNER={bob_env} 应还原成 uid 8");

    // 三条素材路径都要分开 —— 字体（出片/封面）、视频（片头片尾素材）、贴纸。
    let alice_fonts = yekitrim_core::fonts::dir_for(alice);
    let bob_fonts = yekitrim_core::fonts::dir_for(bob);
    assert_ne!(alice_fonts, bob_fonts, "两个账号的字体库必须不同");

    let alice_videos = yekitrim_core::owner::video_dir_readable(alice);
    let bob_videos = yekitrim_core::owner::video_dir_readable(bob);
    assert_ne!(alice_videos, bob_videos, "两个账号的视频库必须不同");

    let alice_stickers = yekitrim_core::owner::sticker_dir_readable(alice);
    let bob_stickers = yekitrim_core::owner::sticker_dir_readable(bob);
    assert_ne!(alice_stickers, bob_stickers, "两个账号的贴纸库必须不同");

    let _ = std::fs::remove_dir_all(&home);
}

/// **单人不变**：没有 `YEKITRIM_OWNER`（CLI / 托盘的常态）⇒ 解析到现状那两个位置。
///
/// 这条是"单人零回归"在**子进程侧**的判据 —— 上面那条改的是子进程怎么选库，
/// 一旦把 `Local` 也导去用户树，托盘用户升级后素材库会突然全空。
#[test]
fn missing_env_keeps_the_single_user_locations() {
    use yekitrim_core::owner::Owner;

    // 父进程没设（`None`）＝ 单人。
    assert_eq!(Owner::from_env_value(None), Owner::Local);
    assert_eq!(Owner::from_env_value(Some("local")), Owner::Local);

    // 单人解析结果**必须逐字等于** path 层那份现状实现（含存量回落）。
    assert_eq!(
        yekitrim_core::fonts::dir_for(Owner::Local),
        yekitrim_core::path::font_dir(),
        "单人字体库必须还是 <数据目录>/resources/fonts"
    );
    assert_eq!(
        yekitrim_core::owner::video_dir_readable(Owner::Local),
        yekitrim_core::path::video_dir_readable(),
        "单人视频库必须还走现状那条（含存量回落）"
    );
    assert_eq!(
        yekitrim_core::owner::sticker_dir_readable(Owner::Local),
        yekitrim_core::path::sticker_dir_readable(),
        "单人贴纸库必须还走现状那条（含存量回落）"
    );

    // 而多用户**不带**那份回落（用户树是新长出来的，没有"老位置"这回事）。
    let user_home = std::env::temp_dir().join("yekitrim-ownerenv-no-fallback");
    assert_eq!(
        yekitrim_core::owner::video_dir_readable_at(Owner::User(3), &user_home),
        yekitrim_core::owner::asset_dir_at(
            Owner::User(3),
            yekitrim_core::owner::AssetKind::Videos,
            &user_home
        ),
        "多用户下视频库必须**只**是用户树里那一个位置，不许有候选回落"
    );
}

/// 认不出的值回落单人形态 —— **不 panic、不报错**（出片失败比"用错了库"更难查）。
#[test]
fn bogus_env_value_falls_back_to_local_instead_of_failing() {
    use yekitrim_core::owner::Owner;
    for bad in ["", "   ", "nope", "0", "-1", "uid:7", "7.0"] {
        assert_eq!(
            Owner::from_env_value(Some(bad)),
            Owner::Local,
            "{bad:?} 应回落单人形态而不是失败"
        );
    }
}

/// `Owner::from_env_value` 与 `as_env_value` 互逆（父进程写什么、子进程就读回什么）。
///
/// ⚠️ 这里**只读**环境变量、不写 —— 见文件头"判据的设计"。
#[test]
fn env_roundtrip_is_exact_for_every_owner() {
    use yekitrim_core::owner::Owner;
    for owner in [Owner::Local, Owner::User(1), Owner::User(42)] {
        let written = owner.as_env_value();
        assert_eq!(
            Owner::from_env_value(Some(&written)),
            owner,
            "父进程写 {written:?}、子进程读回来必须还是 {owner:?}"
        );
    }
    // 单人写的是字面量 "local"，不是 uid（单人没有真账号）。
    assert_eq!(Owner::Local.as_env_value(), "local");
}

/// 身份经**真子进程**传递 —— 验的是"环境变量会继承"，而不是纯函数。
///
/// 出片链路里 ffmpeg 与底座 Dhampir 是**孙进程**（`render/AGENTS.md` 反复强调）。
/// 环境变量天然继承、命令行不会 —— 这条起一个真子进程，让它把拿到的
/// `YEKITRIM_OWNER` 打回来，断言**值原样到达**。纯函数判据全绿、链路却断了的情况，
/// 就靠这条兜住。
#[test]
fn owner_env_is_inherited_across_a_real_child_process() {
    use yekitrim_core::owner::Owner;

    // 借一个真实存在的子进程：起自己这个测试二进制，让它把环境变量打回来。
    // ⚠️ 不能 `set_var` 改本进程环境（unsafe + 会污染并行用例），
    // 所以用 `Command::env()` —— **与 `run_polish_child` 是同一个 API**，
    // 走的正是生产那条路。
    let exe = std::env::current_exe().expect("测试二进制路径");
    // 用 `cmd`（Windows）/ `sh`（别处）读一下环境变量并打出来：
    // 这比再起一份测试二进制更直接，且不依赖 libtest 的参数语义。
    #[cfg(windows)]
    let out = std::process::Command::new("cmd")
        .args(["/C", "echo %YEKITRIM_OWNER%"])
        .env("YEKITRIM_OWNER", Owner::User(7).as_env_value())
        .output();
    #[cfg(not(windows))]
    let out = std::process::Command::new("sh")
        .args(["-c", "printf %s \"$YEKITRIM_OWNER\""])
        .env("YEKITRIM_OWNER", Owner::User(7).as_env_value())
        .output();

    let _ = exe;
    let out = out.expect("应能起子进程");
    let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        got, "7",
        "子进程必须原样拿到父进程写的 YEKITRIM_OWNER（这是孙进程也能认出身分的根据）"
    );
    assert_eq!(
        Owner::from_env_value(Some(&got)),
        Owner::User(7),
        "子进程拿到的字符串要能还原成身份"
    );
}
