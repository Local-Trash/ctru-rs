/* automatically generated by rust-bindgen */

#![allow(dead_code,
         non_camel_case_types,
         non_upper_case_globals,
         non_snake_case)]
extern "C" {
    pub fn pmInit() -> Result;
    pub fn pmExit();
    pub fn PM_LaunchTitle(mediatype: u8_, titleid: u64_, launch_flags: u32_)
     -> Result;
    pub fn PM_GetTitleExheaderFlags(mediatype: u8_, titleid: u64_,
                                    out: *mut u8_) -> Result;
    pub fn PM_SetFIRMLaunchParams(size: u32_, in_: *mut u8_) -> Result;
    pub fn PM_GetFIRMLaunchParams(size: u32_, out: *mut u8_) -> Result;
    pub fn PM_LaunchFIRMSetParams(firm_titleid_low: u32_, size: u32_,
                                  in_: *mut u8_) -> Result;
}
use ::types::*;