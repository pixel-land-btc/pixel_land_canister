use crate::canvas::pixel::{AccountId, Pixel, Rgb888};

#[derive(Clone)]
pub struct Canvas {
	width: usize,
	height: usize,
	// 一维向量存储，按行主序（row-major）：idx = x + y * width
	pixels: Vec<Pixel>,
}

/// 与画布相关的错误类型
#[derive(Debug)]
pub enum CanvasError {
	OutOfBounds,
	PriceTooLow { required: u128 },
}

impl Canvas {
	/// 创建画布：所有像素初始无主、指定初始价、默认颜色 (#FFFFFF)。
	pub fn new(width: usize, height: usize, initial_price: u128) -> Self {
		let default_pixel = Pixel {
			owner: None,
			price: initial_price,
			color: Rgb888(0xFFFFFF),
		};
		Self {
			width,
			height,
			pixels: vec![default_pixel; width * height],
		}
	}
	
	/// 将 (x,y) 坐标映射到vec索引
	fn idx(&self, x: usize, y: usize) -> Result<usize, CanvasError> {
		if x < self.width && y < self.height {
			Ok(x + y * self.width)
		} else {
			Err(CanvasError::OutOfBounds)
		}
	}
	
	/// 读取像素
	pub fn pixel(&self, x: usize, y: usize) -> Result<&Pixel, CanvasError> {
		self.idx(x, y).map(|i| &self.pixels[i])
	}
	
	/// **内部函数**：可变引用（封装成公共业务函数更安全）
	fn pixel_mut(&mut self, x: usize, y: usize) -> Result<&mut Pixel, CanvasError> {
		self.idx(x, y).map(|i| &mut self.pixels[i])
	}
	
	// ─── 业务接口 ───────────────────────
	
	/// 仅改变颜色，不涉及价格与 ownership
	pub fn set_color(&mut self, x: usize, y: usize, color: Rgb888) -> Result<(), CanvasError> {
		self.pixel_mut(x, y)?.color = color;
		Ok(())
	}
	
	/// 购买像素：支付金额需 ≥ 当前价；成功后
	///   * 所有权转移
	///   * 像素价格可按策略上调（下例简单翻倍，可自行改为 +Δ 或乘常数）
	///   * 同时设置像素颜色
	///
	/// 在链上时应由调用方完成余额扣减 / 资产转移，再回调此逻辑。
	pub fn buy_pixel(
		&mut self,
		x: usize,
		y: usize,
		buyer: AccountId,
		amount_paid: u128,
		new_color: Rgb888,
	) -> Result<(), CanvasError> {
		let pix = self.pixel_mut(x, y)?;
		
		if amount_paid < pix.price {
			return Err(CanvasError::PriceTooLow {
				required: pix.price,
			});
		}
		
		pix.owner = Some(buyer);
		pix.color = new_color;
		pix.price = Self::next_price(pix.price);
		Ok(())
	}
	
	/// 定义价格递增策略（示例：*2）
	fn next_price(current: u128) -> u128 {
		current.saturating_mul(2)
	}
}