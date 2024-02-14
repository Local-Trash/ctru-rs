//! Software Keyboard applet.
//!
//! This applet opens a virtual keyboard on the console's bottom screen which lets the user write UTF-16 valid text.
#![doc(alias = "keyboard")]

use crate::services::{apt::Apt, gfx::Gfx};
use ctru_sys::{
    self, aptLaunchLibraryApplet, aptSetMessageCallback, envGetAptAppId, svcBreak, svcCloseHandle,
    svcCreateMemoryBlock, APT_SendParameter, SwkbdButton, SwkbdDictWord, SwkbdExtra,
    SwkbdLearningData, SwkbdState, SwkbdStatusData, APPID_SOFTWARE_KEYBOARD, APTCMD_MESSAGE,
    NS_APPID, SWKBD_CALLBACK_OK, USERBREAK_PANIC,
};

use bitflags::bitflags;
use libc;

use std::ffi::{CStr, CString};
use std::fmt::Display;
use std::iter::once;
use std::str;

type CallbackFunction = dyn Fn(&CStr) -> (CallbackResult, Option<CString>);

// I hate that we have to use this, but sometimes you gotta smuggle pointers into C callbacks
// and that's just how things are
static mut SWKBD_SHARED_MEM: *mut libc::c_void = std::ptr::null_mut();

/// Configuration structure to setup the Software Keyboard applet.
#[doc(alias = "SwkbdState")]
pub struct SoftwareKeyboard {
    state: Box<SwkbdState>,
    callback: Option<Box<CallbackFunction>>,
    error_message: Option<CString>,
    initial_text: Option<CString>,
}

/// Configuration structure to setup the Parental Lock applet.
///
/// Internally, the Parental Lock is just a different kind of [`SoftwareKeyboard`].
#[doc(alias = "SwkbdState")]
#[derive(Clone)]
pub struct ParentalLock {
    state: Box<SwkbdState>,
}

/// The type of keyboard used by the [`SoftwareKeyboard`].
///
/// Can be set with [`SoftwareKeyboard::new()`]
#[doc(alias = "SwkbdType")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    /// Normal keyboard composed of several pages (QWERTY, accents, symbols, mobile).
    Normal = ctru_sys::SWKBD_TYPE_NORMAL,
    /// Only QWERTY keyboard.
    Qwerty = ctru_sys::SWKBD_TYPE_QWERTY,
    /// Only number pad.
    Numpad = ctru_sys::SWKBD_TYPE_NUMPAD,
    /// On JPN systems: a keyboard without japanese input capabilities.
    ///
    /// On any other region: same as [`Normal`](Kind::Normal).
    Western = ctru_sys::SWKBD_TYPE_WESTERN,
}

/// The type of result returned by a custom filter callback.
///
/// The custom callback can be set using [`SoftwareKeyboard::set_filter_callback()`].
#[doc(alias = "SwkbdCallbackResult")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CallbackResult {
    /// The callback yields a positive result.
    Ok = ctru_sys::SWKBD_CALLBACK_OK,
    /// The callback finds the input invalid, but lets the user try again.
    Retry = ctru_sys::SWKBD_CALLBACK_CONTINUE,
    /// The callback finds the input invalid and closes the Software Keyboard view.
    Close = ctru_sys::SWKBD_CALLBACK_CLOSE,
}

/// Represents which button the user pressed to close the [`SoftwareKeyboard`].
///
/// Button text and behaviour can be customized with [`SoftwareKeyboard::configure_button()`].
#[doc(alias = "SwkbdButton")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Button {
    /// Left button. Usually corresponds to "Cancel".
    Left = ctru_sys::SWKBD_BUTTON_LEFT,
    /// Middle button. Usually corresponds to "I Forgot".
    Middle = ctru_sys::SWKBD_BUTTON_MIDDLE,
    /// Right button. Usually corresponds to "OK".
    Right = ctru_sys::SWKBD_BUTTON_RIGHT,
}

/// Represents the password mode to conceal the input text for the [`SoftwareKeyboard`].
///
/// Can be set using [`SoftwareKeyboard::set_password_mode()`].
#[doc(alias = "SwkbdPasswordMode")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PasswordMode {
    /// The input text will not be concealed.
    None = ctru_sys::SWKBD_PASSWORD_NONE,
    /// The input text will be concealed immediately after typing.
    Hide = ctru_sys::SWKBD_PASSWORD_HIDE,
    /// The input text will be concealed a second after typing.
    HideDelay = ctru_sys::SWKBD_PASSWORD_HIDE_DELAY,
}

/// Configuration to setup the on-screen buttons to exit the [`SoftwareKeyboard`] prompt.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ButtonConfig {
    /// 1 Button: considered the right button.
    Right = 1,
    /// 2 Buttons: left and right buttons.
    LeftRight = 2,
    /// 3 Buttons: left, middle and right buttons.
    LeftMiddleRight = 3,
}

/// Error returned by an unsuccessful [`SoftwareKeyboard::get_string()`].
#[doc(alias = "SwkbdResult")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Error {
    /// Invalid parameters given to the [`SoftwareKeyboard`] configuration.
    InvalidParameters = ctru_sys::SWKBD_INVALID_INPUT,
    /// [`SoftwareKeyboard`] ran out of memory.
    OutOfMem = ctru_sys::SWKBD_OUTOFMEM,
    /// Home button was pressed while [`SoftwareKeyboard`] was running.
    HomePressed = ctru_sys::SWKBD_HOMEPRESSED,
    /// Reset button was pressed while [`SoftwareKeyboard`] was running.
    ResetPressed = ctru_sys::SWKBD_RESETPRESSED,
    /// Power button was pressed while [`SoftwareKeyboard`] was running.
    PowerPressed = ctru_sys::SWKBD_POWERPRESSED,
    /// The parental lock PIN was correct.
    ///
    /// This variant should never be returned by normal operations made using this module,
    /// and is listed here only for compatibility purposes.
    /// Refer to the return value of [`ParentalLock::launch()`] to confirm the outcome
    /// of the Parental Lock PIN operation.
    ParentalOk = ctru_sys::SWKBD_PARENTAL_OK,
    /// The parental lock PIN was incorrect.
    ///
    /// Refer to the return value of [`ParentalLock::launch()`] to confirm the outcome
    /// of the Parental Lock PIN operation.
    ParentalFail = ctru_sys::SWKBD_PARENTAL_FAIL,
    /// Input triggered the filter.
    ///
    /// You can have a look at [`Filters`] to activate custom filters.
    BannedInput = ctru_sys::SWKBD_BANNED_INPUT,
    /// An on-screen button was pressed to exit the prompt.
    ButtonPressed = ctru_sys::SWKBD_D0_CLICK,
}

/// Restrictions to enforce rules on the keyboard input.
///
/// See [`SoftwareKeyboard::set_validation()`]
#[doc(alias = "SwkbdValidInput")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ValidInput {
    /// All inputs are accepted.
    Anything = ctru_sys::SWKBD_ANYTHING,
    /// Empty inputs are not accepted.
    NotEmpty = ctru_sys::SWKBD_NOTEMPTY,
    /// Blank inputs (consisting only of whitespaces) are not accepted.
    NotBlank = ctru_sys::SWKBD_NOTBLANK,
    /// Neither empty inputs nor blank inputs are accepted.
    NotEmptyNotBlank = ctru_sys::SWKBD_NOTEMPTY_NOTBLANK,
    /// Input must have a fixed length. Maximum length can be specified with [`SoftwareKeyboard::set_max_text_len()`];
    FixedLen = ctru_sys::SWKBD_FIXEDLEN,
}

bitflags! {
    /// Special features that can be activated via [`SoftwareKeyboard::set_features()`].
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
    pub struct Features: u32 {
        /// Darken top screen while the [`SoftwareKeyboard`] is active.
        const DARKEN_TOP_SCREEN = ctru_sys::SWKBD_DARKEN_TOP_SCREEN;
        /// Enable predictive input (necessary for Kanji on JPN consoles).
        const PREDICTIVE_INPUT  = ctru_sys::SWKBD_PREDICTIVE_INPUT;
        /// Enable multiline input.
        const MULTILINE         = ctru_sys::SWKBD_MULTILINE;
        /// Enable fixed-width mode.
        const FIXED_WIDTH       = ctru_sys::SWKBD_FIXED_WIDTH;
        /// Allow the usage of the Home Button while the [`SoftwareKeyboard`] is running.
        const ALLOW_HOME        = ctru_sys::SWKBD_ALLOW_HOME;
        /// Allow the usage of the Reset Button while the [`SoftwareKeyboard`] is running.
        const ALLOW_RESET       = ctru_sys::SWKBD_ALLOW_RESET;
        /// Allow the usage of the Power Button while the [`SoftwareKeyboard`] is running.
        const ALLOW_POWER       = ctru_sys::SWKBD_ALLOW_POWER;
        /// Default to the QWERTY page when the [`SoftwareKeyboard`] is shown.
        const DEFAULT_QWERTY    = ctru_sys::SWKBD_DEFAULT_QWERTY;
    }

    /// Availble filters to disallow some types of input for the [`SoftwareKeyboard`].
    ///
    /// See [`SoftwareKeyboard::set_validation()`]
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
    pub struct Filters: u32 {
        /// Disallow the usage of numerical digits.
        ///
        /// The maximum number of digits that are allowed to be used while this filter is active
        /// can be configured with [`SoftwareKeyboard::set_max_digits()`] (default is 0).
        const DIGITS    = ctru_sys::SWKBD_FILTER_DIGITS;
        /// Disallow the usage of the "at" (@) sign.
        const AT        = ctru_sys::SWKBD_FILTER_AT;
        /// Disallow the usage of the "percent" (%) sign.
        const PERCENT   = ctru_sys::SWKBD_FILTER_PERCENT;
        /// Disallow the usage of the "backslash" (\) sign.
        const BACKSLASH = ctru_sys::SWKBD_FILTER_BACKSLASH;
        /// Disallow the use of profanity via Nintendo's profanity filter.
        const PROFANITY = ctru_sys::SWKBD_FILTER_PROFANITY;
    }
}

impl SoftwareKeyboard {
    /// Initialize a new configuration for the Software Keyboard applet depending on how many "exit" buttons are available to the user (1, 2 or 3).
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, ButtonConfig, Kind};
    ///
    /// // Standard keyboard. Equivalent to `SoftwareKeyboard::default()`.
    /// let keyboard = SoftwareKeyboard::new(Kind::Normal, ButtonConfig::LeftRight);
    ///
    /// // Numpad (with only the "confirm" button).
    /// let keyboard = SoftwareKeyboard::new(Kind::Numpad, ButtonConfig::Right);
    /// #
    /// # }
    #[doc(alias = "swkbdInit")]
    pub fn new(keyboard_type: Kind, buttons: ButtonConfig) -> Self {
        unsafe {
            let mut state = Box::<SwkbdState>::default();
            ctru_sys::swkbdInit(state.as_mut(), keyboard_type.into(), buttons.into(), -1);
            Self {
                state,
                callback: None,
                error_message: None,
                initial_text: None,
            }
        }
    }

    /// Launches the applet based on the given configuration and returns a string containing the text input.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # use std::error::Error;
    /// # fn main() -> Result<(), Box<dyn Error>> {
    /// # use ctru::services::{apt::Apt, gfx::Gfx};
    /// #
    /// # let gfx = Gfx::new().unwrap();
    /// # let apt = Apt::new().unwrap();
    /// #
    /// use ctru::applets::swkbd::SoftwareKeyboard;
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// let (text, button) = keyboard.get_string(&apt, &gfx)?;
    /// #
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "swkbdInputText")]
    pub fn get_string(&mut self, _apt: &Apt, _gfx: &Gfx) -> Result<(String, Button), Error> {
        let mut output = String::new();

        unsafe {
            // The filter callback gets reset every time the SoftwareKeyboard is used.
            ctru_sys::swkbdSetFilterCallback(
                self.state.as_mut(),
                Some(Self::internal_callback),
                (self as *mut Self).cast(),
            );

            match Self::swkbd_input_text(self.state.as_mut(), output.as_mut_vec()) {
                ctru_sys::SWKBD_BUTTON_NONE => Err(self.state.result.into()),
                ctru_sys::SWKBD_BUTTON_LEFT => Ok((output, Button::Left)),
                ctru_sys::SWKBD_BUTTON_MIDDLE => Ok((output, Button::Middle)),
                ctru_sys::SWKBD_BUTTON_RIGHT => Ok((output, Button::Right)),
                _ => unreachable!(),
            }
        }
    }

    /// Set special features for this keyboard.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, Features};
    ///
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// let features = Features::DARKEN_TOP_SCREEN & Features::MULTILINE;
    /// keyboard.set_features(features);
    /// #
    /// # }
    #[doc(alias = "swkbdSetFeatures")]
    pub fn set_features(&mut self, features: Features) {
        unsafe { ctru_sys::swkbdSetFeatures(self.state.as_mut(), features.bits()) }
    }

    /// Configure input validation for this keyboard.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, ValidInput, Filters};
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// // Disallow empty or blank input.
    /// let validation = ValidInput::NotEmptyNotBlank;
    ///
    /// // Disallow the use of numerical digits and profanity.
    /// let filters = Filters::DIGITS & Filters::PROFANITY;
    /// keyboard.set_validation(validation, filters);
    /// #
    /// # }
    pub fn set_validation(&mut self, validation: ValidInput, filters: Filters) {
        self.state.valid_input = validation.into();
        self.state.filter_flags = filters.bits();
    }

    /// Configure a custom filtering function to validate the input.
    ///
    /// The callback function must return a [`CallbackResult`] and the error message to display when the input is invalid.
    ///
    /// # Notes
    ///
    /// Passing [`None`] will unbind the custom filter callback.
    ///
    /// The error message returned by the callback should be shorter than `256` characters, otherwise it will be truncated.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use std::borrow::Cow;
    /// use std::ffi::CString;
    /// use ctru::applets::swkbd::{SoftwareKeyboard, CallbackResult};
    ///
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// keyboard.set_filter_callback(Some(Box::new(|str| {
    ///     if str.to_str().unwrap().contains("boo") {
    ///         return (
    ///             CallbackResult::Retry,
    ///             Some(CString::new("Ah, you scared me!").unwrap()),
    ///         );
    ///     }
    ///
    ///     (CallbackResult::Ok, None)
    /// })));
    /// #
    /// # }
    pub fn set_filter_callback(&mut self, callback: Option<Box<CallbackFunction>>) {
        self.callback = callback;
    }

    /// Internal function called by the filter callback.
    extern "C" fn internal_callback(
        user: *mut libc::c_void,
        pp_message: *mut *const libc::c_char,
        text: *const libc::c_char,
        _text_size: libc::size_t,
    ) -> ctru_sys::SwkbdCallbackResult {
        let this: *mut SoftwareKeyboard = user.cast();

        unsafe {
            // Reset any leftover error message.
            (*this).error_message = None;

            let text = CStr::from_ptr(text);

            let result = {
                // Run the callback if still available.
                if let Some(callback) = &mut (*this).callback {
                    let (res, cstr) = callback(text);

                    // Due to how `libctru` operates, the user is expected to keep the error message alive until
                    // the end of the Software Keyboard prompt. We ensure that happens by saving it within the configuration.
                    (*this).error_message = cstr;

                    if let Some(newstr) = &(*this).error_message {
                        *pp_message = newstr.as_ptr();
                    }

                    res
                } else {
                    CallbackResult::Ok
                }
            };

            result.into()
        }
    }

    /// Configure the maximum number of digits that can be entered in the keyboard when the [`Filters::DIGITS`] flag is enabled.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, ValidInput, Filters};
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// // Disallow empty or blank input.
    /// let validation = ValidInput::NotEmptyNotBlank;
    ///
    /// // Disallow the use of numerical digits. This filter is customizable thanks to `set_max_digits`.
    /// let filters = Filters::DIGITS;
    /// keyboard.set_validation(validation, filters);
    ///
    /// // No more than 3 numbers are accepted.
    /// keyboard.set_max_digits(3);
    /// #
    /// # }
    pub fn set_max_digits(&mut self, digits: u16) {
        self.state.max_digits = digits;
    }

    /// Set the initial text for this software keyboard.
    ///
    /// The initial text is the text already written when you open the software keyboard.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::SoftwareKeyboard;
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// keyboard.set_initial_text(Some("Write here what you like!"));
    /// #
    /// # }
    #[doc(alias = "swkbdSetInitialText")]
    pub fn set_initial_text(&mut self, text: Option<&str>) {
        if let Some(text) = text {
            let initial_text = CString::new(text).unwrap();

            unsafe {
                ctru_sys::swkbdSetInitialText(self.state.as_mut(), initial_text.as_ptr());
            }

            self.initial_text = Some(initial_text);
        } else {
            unsafe { ctru_sys::swkbdSetInitialText(self.state.as_mut(), std::ptr::null()) };

            self.initial_text = None;
        }
    }

    /// Set the hint text for this software keyboard.
    ///
    /// The hint text is the text shown in gray before any text gets written in the input box.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::SoftwareKeyboard;
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// keyboard.set_hint_text("Write here what you like!");
    /// #
    /// # }
    #[doc(alias = "swkbdSetHintText")]
    pub fn set_hint_text(&mut self, text: &str) {
        unsafe {
            let nul_terminated: String = text.chars().chain(once('\0')).collect();
            ctru_sys::swkbdSetHintText(self.state.as_mut(), nul_terminated.as_ptr());
        }
    }

    /// Set a password mode for this software keyboard.
    ///
    /// Depending on the selected mode the input text will be concealed.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, PasswordMode};
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// keyboard.set_password_mode(PasswordMode::Hide);
    /// #
    /// # }
    #[doc(alias = "swkbdSetPasswordMode")]
    pub fn set_password_mode(&mut self, mode: PasswordMode) {
        unsafe {
            ctru_sys::swkbdSetPasswordMode(self.state.as_mut(), mode.into());
        }
    }

    /// Set the 2 custom characters to add to the keyboard while using [`Kind::Numpad`].
    ///
    /// These characters will appear in their own buttons right next to the `0` key.
    ///
    /// # Notes
    ///
    /// If [`None`] is passed as either key, that button will not be shown to the user.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, Kind, ButtonConfig};
    /// let mut keyboard = SoftwareKeyboard::new(Kind::Numpad, ButtonConfig::LeftRight);
    ///
    /// keyboard.set_numpad_keys(Some('#'), Some('.'));
    ///
    /// // The right numpad key will not be shown.
    /// keyboard.set_numpad_keys(Some('!'), None);
    /// #
    /// # }
    #[doc(alias = "swkbdSetNumpadKeys")]
    pub fn set_numpad_keys(&mut self, left_key: Option<char>, right_key: Option<char>) {
        let mut keys = (0, 0);

        if let Some(k) = left_key {
            keys.0 = k as i32;
        }

        if let Some(k) = right_key {
            keys.1 = k as i32;
        }

        unsafe {
            ctru_sys::swkbdSetNumpadKeys(self.state.as_mut(), keys.0, keys.1);
        }
    }

    /// Configure the look and behavior of a button for this keyboard.
    ///
    /// # Arguments
    ///
    /// - `button` - the [`Button`] to be configured based on the position.
    /// - `text` - the text displayed in the button.
    /// - `submit` - whether pressing the button will accept the keyboard's input or discard it.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, Button, ButtonConfig, Kind};
    ///
    /// // We create a `SoftwareKeyboard` with left and right buttons.
    /// let mut keyboard = SoftwareKeyboard::new(Kind::Normal, ButtonConfig::LeftRight);
    ///
    /// // Set the left button text to "Cancel" and pressing it will NOT return the user's input.
    /// keyboard.configure_button(Button::Left, "Cancel", false);
    ///
    /// // Set the right button text to "Ok" and pressing it will return the user's input.
    /// keyboard.configure_button(Button::Right, "Ok", true);
    /// #
    /// # }
    #[doc(alias = "swkbdSetButton")]
    pub fn configure_button(&mut self, button: Button, text: &str, submit: bool) {
        unsafe {
            let nul_terminated: String = text.chars().chain(once('\0')).collect();
            ctru_sys::swkbdSetButton(
                self.state.as_mut(),
                button.into(),
                nul_terminated.as_ptr(),
                submit,
            );
        }
    }

    /// Configure the maximum number of UTF-16 code units that can be entered into the software
    /// keyboard. By default the limit is `65000` code units.
    ///
    /// # Notes
    ///
    /// This action will overwrite any previously submitted [`ValidInput`] validation.
    ///
    /// Keyboard input is converted from UTF-16 to UTF-8 before being handed to Rust,
    /// so this code point limit does not necessarily equal the max number of UTF-8 code points
    /// receivable by [`SoftwareKeyboard::get_string()`] and [`SoftwareKeyboard::write_exact()`].
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::{SoftwareKeyboard, Button, Kind};
    /// let mut keyboard = SoftwareKeyboard::default();
    ///
    /// // Set the maximum text length to 18 UTF-16 code units.
    /// keyboard.set_max_text_len(18);
    /// #
    /// # }
    pub fn set_max_text_len(&mut self, len: u16) {
        self.state.max_text_len = len;

        // Activate the specific validation rule for maximum length.
        self.state.valid_input = ValidInput::FixedLen.into();
    }

    // A reimplementation of `swkbdInputText` from `libctru/source/applets/swkbd.c`. Allows us to
    // get text from the software keyboard and put it directly into a `String` without requiring
    // an intermediate fixed-size buffer
    fn swkbd_input_text(swkbd: &mut SwkbdState, buf: &mut Vec<u8>) -> SwkbdButton {
        use ctru_sys::{
            MEMPERM_READ, MEMPERM_WRITE, R_FAILED, SWKBD_BUTTON_LEFT, SWKBD_BUTTON_MIDDLE,
            SWKBD_BUTTON_NONE, SWKBD_BUTTON_RIGHT, SWKBD_D0_CLICK, SWKBD_D1_CLICK0,
            SWKBD_D1_CLICK1, SWKBD_D2_CLICK0, SWKBD_D2_CLICK1, SWKBD_D2_CLICK2,
            SWKBD_FILTER_CALLBACK, SWKBD_OUTOFMEM,
        };

        let mut extra = unsafe { swkbd.__bindgen_anon_1.extra };

        // Calculate shared mem size
        let mut shared_mem_size = 0;

        shared_mem_size +=
            (std::mem::size_of::<u16>() * (swkbd.max_text_len as usize + 1) + 3) & !3;

        let dict_off = shared_mem_size;

        shared_mem_size +=
            (std::mem::size_of::<SwkbdDictWord>() * swkbd.dict_word_count as usize + 3) & !3;

        let status_off = shared_mem_size;

        shared_mem_size += if swkbd.initial_learning_offset >= 0 {
            std::mem::size_of::<SwkbdStatusData>()
        } else {
            0
        };

        let learning_off = shared_mem_size;

        shared_mem_size += if swkbd.initial_learning_offset >= 0 {
            std::mem::size_of::<SwkbdLearningData>()
        } else {
            0
        };

        if swkbd.save_state_flags & (1 << 0) != 0 {
            swkbd.status_offset = shared_mem_size as _;
            shared_mem_size += std::mem::size_of::<SwkbdStatusData>();
        }

        if swkbd.save_state_flags & (1 << 1) != 0 {
            swkbd.learning_offset = shared_mem_size as _;
            shared_mem_size += std::mem::size_of::<SwkbdLearningData>();
        }

        shared_mem_size = (shared_mem_size + 0xFFF) & !0xFFF;

        swkbd.shared_memory_size = shared_mem_size;

        // Allocate shared mem
        unsafe { SWKBD_SHARED_MEM = libc::memalign(0x1000, shared_mem_size).cast() };

        let mut swkbd_shared_mem_handle = 0;

        if unsafe { SWKBD_SHARED_MEM.is_null() } {
            swkbd.result = SWKBD_OUTOFMEM;
            return SWKBD_BUTTON_NONE;
        }

        let res = unsafe {
            svcCreateMemoryBlock(
                &mut swkbd_shared_mem_handle,
                SWKBD_SHARED_MEM as _,
                shared_mem_size as _,
                MEMPERM_READ | MEMPERM_WRITE,
                MEMPERM_READ | MEMPERM_WRITE,
            )
        };

        if R_FAILED(res) {
            unsafe {
                libc::free(SWKBD_SHARED_MEM);
                swkbd.result = SWKBD_OUTOFMEM;
                return SWKBD_BUTTON_NONE;
            }
        }

        // Copy stuff to shared mem
        if !extra.initial_text.is_null() {
            swkbd.initial_text_offset = 0;

            unsafe {
                let utf16_iter = CStr::from_ptr(extra.initial_text)
                    .to_str()
                    .unwrap()
                    .encode_utf16()
                    .chain(once(0));

                let mut initial_text_cursor = SWKBD_SHARED_MEM.cast::<u16>();

                for ch in utf16_iter {
                    *initial_text_cursor = ch;
                    initial_text_cursor = initial_text_cursor.add(1);
                }
            }
        }

        if !extra.dict.is_null() {
            swkbd.dict_offset = dict_off as _;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    extra.dict,
                    SWKBD_SHARED_MEM.add(dict_off).cast(),
                    swkbd.dict_word_count as _,
                )
            };
        }

        if swkbd.initial_status_offset >= 0 {
            swkbd.initial_status_offset = status_off as _;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    extra.status_data,
                    SWKBD_SHARED_MEM.add(status_off).cast(),
                    1,
                )
            };
        }

        if swkbd.initial_learning_offset >= 0 {
            swkbd.initial_learning_offset = learning_off as _;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    extra.learning_data,
                    SWKBD_SHARED_MEM.add(learning_off).cast(),
                    1,
                )
            };
        }

        if extra.callback.is_some() {
            swkbd.filter_flags |= SWKBD_FILTER_CALLBACK;
        } else {
            swkbd.filter_flags &= !SWKBD_FILTER_CALLBACK;
        }

        // Launch swkbd
        unsafe {
            swkbd.__bindgen_anon_1.reserved.fill(0);

            if extra.callback.is_some() {
                aptSetMessageCallback(
                    Some(Self::swkbd_message_callback),
                    std::ptr::addr_of_mut!(extra).cast(),
                );
            }

            aptLaunchLibraryApplet(
                APPID_SOFTWARE_KEYBOARD,
                swkbd as *mut _ as *mut _,
                std::mem::size_of::<SwkbdState>(),
                swkbd_shared_mem_handle,
            );

            if extra.callback.is_some() {
                aptSetMessageCallback(None, std::ptr::null_mut());
            }

            let _ = svcCloseHandle(swkbd_shared_mem_handle);
        }

        let button = match swkbd.result {
            SWKBD_D1_CLICK0 | SWKBD_D2_CLICK0 => SWKBD_BUTTON_LEFT,
            SWKBD_D2_CLICK1 => SWKBD_BUTTON_MIDDLE,
            SWKBD_D0_CLICK | SWKBD_D1_CLICK1 | SWKBD_D2_CLICK2 => SWKBD_BUTTON_RIGHT,
            _ => SWKBD_BUTTON_NONE,
        };

        let text16 = if swkbd.text_length > 0 {
            unsafe {
                widestring::Utf16Str::from_slice(std::slice::from_raw_parts(
                    SWKBD_SHARED_MEM.add(swkbd.text_offset as _).cast::<u16>(),
                    swkbd.text_length as usize,
                ))
                .unwrap()
            }
        } else {
            widestring::utf16str!("")
        };

        buf.extend(text16.encode_utf8());

        if swkbd.save_state_flags & (1 << 0) != 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    SWKBD_SHARED_MEM.add(swkbd.status_offset as _).cast(),
                    extra.status_data,
                    1,
                )
            };
        }

        if swkbd.save_state_flags & (1 << 1) != 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    SWKBD_SHARED_MEM.add(swkbd.learning_offset as _).cast(),
                    extra.learning_data,
                    1,
                )
            };
        }

        unsafe { libc::free(SWKBD_SHARED_MEM) };

        button
    }

    // A reimplementation of `swkbdMessageCallback` from `libctru/source/applets/swkbd.c`.
    // This is only needed because the original function is private to libctru, so we can't
    // simply reuse their version
    #[allow(non_snake_case)]
    unsafe extern "C" fn swkbd_message_callback(
        user: *mut libc::c_void,
        sender: NS_APPID,
        msg: *mut libc::c_void,
        msg_size: libc::size_t,
    ) {
        let extra = &mut *user.cast::<SwkbdExtra>();
        let swkbd = &mut *msg.cast::<SwkbdState>();

        if sender != ctru_sys::APPID_SOFTWARE_KEYBOARD
            || msg_size != std::mem::size_of::<SwkbdState>()
        {
            return;
        }

        let text16 = unsafe {
            widestring::Utf16Str::from_slice(std::slice::from_raw_parts(
                SWKBD_SHARED_MEM.add(swkbd.text_offset as _).cast::<u16>(),
                swkbd.text_length as usize + 1,
            ))
            .unwrap()
        };

        let text8 = match String::from_utf8(text16.encode_utf8().collect()) {
            Ok(s) => s,
            Err(_) => {
                svcBreak(USERBREAK_PANIC);
                unreachable!();
            }
        };

        let mut retmsg = std::ptr::null();

        swkbd.callback_result = extra.callback.unwrap()(
            extra.callback_user,
            &mut retmsg,
            text8.as_ptr(),
            text8.len(),
        ) as _;

        let retmsg = if !retmsg.is_null() {
            unsafe {
                let len = libc::strlen(retmsg);
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(retmsg, len + 1))
            }
        } else {
            "\0"
        };

        let callback_msg = &mut swkbd.callback_msg;

        if swkbd.callback_result > SWKBD_CALLBACK_OK as _ {
            for (idx, ch) in retmsg.encode_utf16().take(callback_msg.len()).enumerate() {
                callback_msg[idx] = ch;
            }
        } else {
            callback_msg[0] = 0;
        }

        let _ = APT_SendParameter(
            envGetAptAppId(),
            sender,
            APTCMD_MESSAGE,
            swkbd as *mut _ as *mut _,
            std::mem::size_of::<SwkbdState>() as _,
            0,
        );
    }
}

impl ParentalLock {
    /// Initialize a new configuration for the Parental Lock applet.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// #
    /// use ctru::applets::swkbd::ParentalLock;
    ///
    /// let parental_lock = ParentalLock::new();
    /// #
    /// # }
    #[doc(alias = "swkbdInit")]
    pub fn new() -> Self {
        unsafe {
            let mut state = Box::<SwkbdState>::default();
            ctru_sys::swkbdInit(state.as_mut(), Kind::Normal.into(), 1, -1);
            ctru_sys::swkbdSetFeatures(state.as_mut(), ctru_sys::SWKBD_PARENTAL);
            Self { state }
        }
    }

    /// Launch the Parental Lock applet based on the configuration and return a result depending on whether the operation was successful or not.
    ///
    /// # Example
    ///
    /// ```
    /// # let _runner = test_runner::GdbRunner::default();
    /// # fn main() {
    /// # use ctru::services::{apt::Apt, gfx::Gfx};
    /// #
    /// # let gfx = Gfx::new().unwrap();
    /// # let apt = Apt::new().unwrap();
    /// use ctru::applets::swkbd::{ParentalLock, Error};
    ///
    /// let mut parental_lock = ParentalLock::new();
    ///
    /// match parental_lock.launch(&apt, &gfx) {
    ///     Ok(_) => println!("You can access parental-only features and settings."),
    ///     Err(Error::ParentalFail) => println!("Is a kid trying to access this?"),
    ///     Err(_) => println!("Something wrong happened during the parental lock prompt.")
    /// }
    /// #
    /// # }
    #[doc(alias = "swkbdInputText")]
    pub fn launch(&mut self, _apt: &Apt, _gfx: &Gfx) -> Result<(), Error> {
        unsafe {
            let mut buf = [0; 0];
            ctru_sys::swkbdInputText(self.state.as_mut(), buf.as_mut_ptr(), 0);
            let e = self.state.result.into();

            match e {
                Error::ParentalOk => Ok(()),
                _ => Err(e),
            }
        }
    }
}

/// Creates a new [`SoftwareKeyboard`] configuration set to using a [`Kind::Normal`] keyboard and 2 [`Button`]s.
impl Default for SoftwareKeyboard {
    fn default() -> Self {
        SoftwareKeyboard::new(Kind::Normal, ButtonConfig::LeftRight)
    }
}

impl Default for ParentalLock {
    fn default() -> Self {
        ParentalLock::new()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParameters => write!(
                f,
                "software keyboard was configured with invalid parameters"
            ),
            Self::OutOfMem => write!(f, "software keyboard ran out of memory"),
            Self::HomePressed => {
                write!(f, "home button pressed while software keyboard was running")
            }
            Self::ResetPressed => write!(
                f,
                "reset button pressed while software keyboard was running"
            ),
            Self::PowerPressed => write!(
                f,
                "power button pressed while software keyboard was running"
            ),
            Self::ParentalOk => write!(f, "parental lock pin was correct"),
            Self::ParentalFail => write!(f, "parental lock pin was incorrect"),
            Self::BannedInput => write!(
                f,
                "input given to the software keyboard triggered the active filters"
            ),
            Self::ButtonPressed => write!(f, "on-screen button was pressed to exit the prompt"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ctru_sys::SwkbdResult> for Error {
    fn from(value: ctru_sys::SwkbdResult) -> Self {
        match value {
            ctru_sys::SWKBD_INVALID_INPUT => Error::InvalidParameters,
            ctru_sys::SWKBD_OUTOFMEM => Error::OutOfMem,
            ctru_sys::SWKBD_HOMEPRESSED => Error::HomePressed,
            ctru_sys::SWKBD_RESETPRESSED => Error::ResetPressed,
            ctru_sys::SWKBD_POWERPRESSED => Error::PowerPressed,
            ctru_sys::SWKBD_PARENTAL_OK => Error::ParentalOk,
            ctru_sys::SWKBD_PARENTAL_FAIL => Error::ParentalFail,
            ctru_sys::SWKBD_BANNED_INPUT => Error::BannedInput,
            ctru_sys::SWKBD_D0_CLICK => Error::ButtonPressed,
            ctru_sys::SWKBD_D1_CLICK0 => Error::ButtonPressed,
            ctru_sys::SWKBD_D1_CLICK1 => Error::ButtonPressed,
            ctru_sys::SWKBD_D2_CLICK0 => Error::ButtonPressed,
            ctru_sys::SWKBD_D2_CLICK1 => Error::ButtonPressed,
            ctru_sys::SWKBD_D2_CLICK2 => Error::ButtonPressed,
            _ => unreachable!(),
        }
    }
}

from_impl!(Kind, ctru_sys::SwkbdType);
from_impl!(Button, ctru_sys::SwkbdButton);
from_impl!(Error, ctru_sys::SwkbdResult);
from_impl!(ValidInput, i32);
from_impl!(ValidInput, u32);
from_impl!(ButtonConfig, i32);
from_impl!(PasswordMode, u32);
from_impl!(CallbackResult, u32);
