use std::fmt;

/// 像素点占有者的BTC 地址
pub type AccountId = String;

/// 像素单元
#[derive(Clone)]
pub struct Pixel {
	pub owner: Option<AccountId>, // None 表示无人持有。收入归项目方，为Some则收入归像素占有者。
	pub price: u128,              // 当前标价（最小计价单位，自行决定 Token 精度）
	pub color: Rgb888,            // 24‑bit 颜色
}

/// 24‑bit 颜色封装（0xRRGGBB）
#[derive(Copy, Clone, Eq, PartialEq)]
pub struct Rgb888(pub u32);

impl fmt::Display for Rgb888 {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "#{:06X}", self.0 & 0x00FF_FFFF)
	}
}