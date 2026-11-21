//! 本地诊断：日志目录的有限尾部读取与白名单清洗、诊断包打包。外壳负责定位日志目录、
//! 选择保存位置并生成运行时元数据。
pub mod catalog;
pub mod level;
pub mod logs;
pub mod package;
