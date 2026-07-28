use std::{
    env,
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::{Path, PathBuf},
    ptr,
};

use anyhow::{anyhow, bail, Context, Result};
use libloading::Library;
use serde_json::{json, Value};

type EngineSettingsCreate =
    unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> *mut c_void;
type EngineSettingsDelete = unsafe extern "C" fn(*mut c_void);
type EngineSettingsSetMaxTokens = unsafe extern "C" fn(*mut c_void, c_int);
type EngineSettingsSetCacheDir = unsafe extern "C" fn(*mut c_void, *const c_char);
type EngineCreate = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type EngineDelete = unsafe extern "C" fn(*mut c_void);
type SessionConfigCreate = unsafe extern "C" fn() -> *mut c_void;
type SessionConfigDelete = unsafe extern "C" fn(*mut c_void);
type SessionConfigSetMaxOutputTokens = unsafe extern "C" fn(*mut c_void, c_int);
type ConversationConfigCreate = unsafe extern "C" fn() -> *mut c_void;
type ConversationConfigDelete = unsafe extern "C" fn(*mut c_void);
type ConversationConfigSetSession = unsafe extern "C" fn(*mut c_void, *const c_void);
type ConversationConfigSetSystem = unsafe extern "C" fn(*mut c_void, *const c_char);
type ConversationCreate = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
type ConversationDelete = unsafe extern "C" fn(*mut c_void);
type ConversationOptionalArgsCreate = unsafe extern "C" fn() -> *mut c_void;
type ConversationOptionalArgsDelete = unsafe extern "C" fn(*mut c_void);
type ConversationOptionalArgsSetMaxOutputTokens = unsafe extern "C" fn(*mut c_void, c_int);
type ConversationSendMessage =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_void) -> *mut c_void;
type JsonResponseDelete = unsafe extern "C" fn(*mut c_void);
type JsonResponseGetString = unsafe extern "C" fn(*const c_void) -> *const c_char;
type SetMinLogLevel = unsafe extern "C" fn(c_int);

struct Api {
    _library: Library,
    engine_settings_create: EngineSettingsCreate,
    engine_settings_delete: EngineSettingsDelete,
    engine_settings_set_max_tokens: EngineSettingsSetMaxTokens,
    engine_settings_set_cache_dir: EngineSettingsSetCacheDir,
    engine_create: EngineCreate,
    engine_delete: EngineDelete,
    session_config_create: SessionConfigCreate,
    session_config_delete: SessionConfigDelete,
    session_config_set_max_output_tokens: SessionConfigSetMaxOutputTokens,
    conversation_config_create: ConversationConfigCreate,
    conversation_config_delete: ConversationConfigDelete,
    conversation_config_set_session: ConversationConfigSetSession,
    conversation_config_set_system: ConversationConfigSetSystem,
    conversation_create: ConversationCreate,
    conversation_delete: ConversationDelete,
    conversation_optional_args_create: ConversationOptionalArgsCreate,
    conversation_optional_args_delete: ConversationOptionalArgsDelete,
    conversation_optional_args_set_max_output_tokens: ConversationOptionalArgsSetMaxOutputTokens,
    conversation_send_message: ConversationSendMessage,
    json_response_delete: JsonResponseDelete,
    json_response_get_string: JsonResponseGetString,
    set_min_log_level: SetMinLogLevel,
}

impl Api {
    fn load(path: &Path) -> Result<Self> {
        // SAFETY: The package pins the LiteRT-LM DLL and C header to v0.14.0.
        // Every loaded symbol is copied as a function pointer while `Library`
        // remains owned by this struct for at least as long as those pointers.
        unsafe {
            let library =
                Library::new(path).with_context(|| format!("failed to load {}", path.display()))?;
            Ok(Self {
                engine_settings_create: symbol(&library, b"litert_lm_engine_settings_create\0")?,
                engine_settings_delete: symbol(&library, b"litert_lm_engine_settings_delete\0")?,
                engine_settings_set_max_tokens: symbol(
                    &library,
                    b"litert_lm_engine_settings_set_max_num_tokens\0",
                )?,
                engine_settings_set_cache_dir: symbol(
                    &library,
                    b"litert_lm_engine_settings_set_cache_dir\0",
                )?,
                engine_create: symbol(&library, b"litert_lm_engine_create\0")?,
                engine_delete: symbol(&library, b"litert_lm_engine_delete\0")?,
                session_config_create: symbol(&library, b"litert_lm_session_config_create\0")?,
                session_config_delete: symbol(&library, b"litert_lm_session_config_delete\0")?,
                session_config_set_max_output_tokens: symbol(
                    &library,
                    b"litert_lm_session_config_set_max_output_tokens\0",
                )?,
                conversation_config_create: symbol(
                    &library,
                    b"litert_lm_conversation_config_create\0",
                )?,
                conversation_config_delete: symbol(
                    &library,
                    b"litert_lm_conversation_config_delete\0",
                )?,
                conversation_config_set_session: symbol(
                    &library,
                    b"litert_lm_conversation_config_set_session_config\0",
                )?,
                conversation_config_set_system: symbol(
                    &library,
                    b"litert_lm_conversation_config_set_system_message\0",
                )?,
                conversation_create: symbol(&library, b"litert_lm_conversation_create\0")?,
                conversation_delete: symbol(&library, b"litert_lm_conversation_delete\0")?,
                conversation_optional_args_create: symbol(
                    &library,
                    b"litert_lm_conversation_optional_args_create\0",
                )?,
                conversation_optional_args_delete: symbol(
                    &library,
                    b"litert_lm_conversation_optional_args_delete\0",
                )?,
                conversation_optional_args_set_max_output_tokens: symbol(
                    &library,
                    b"litert_lm_conversation_optional_args_set_max_output_tokens\0",
                )?,
                conversation_send_message: symbol(
                    &library,
                    b"litert_lm_conversation_send_message\0",
                )?,
                json_response_delete: symbol(&library, b"litert_lm_json_response_delete\0")?,
                json_response_get_string: symbol(
                    &library,
                    b"litert_lm_json_response_get_string\0",
                )?,
                set_min_log_level: symbol(&library, b"litert_lm_set_min_log_level\0")?,
                _library: library,
            })
        }
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T> {
    // SAFETY: Callers provide symbol signatures from LiteRT-LM v0.14.0's
    // public C header. The returned pointer cannot outlive `library`.
    Ok(*unsafe { library.get::<T>(name) }?)
}

pub struct LiteRtEngine {
    api: Api,
    engine: *mut c_void,
    backend: &'static str,
    model_name: &'static str,
}

impl LiteRtEngine {
    pub fn load_packaged() -> Result<Self> {
        let runtime_dir = runtime_dir()?;
        let model_path = model_path()?;
        let cache_dir = cache_dir()?;
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("failed to create {}", cache_dir.display()))?;
        let dll_path = runtime_dir.join("litert-lm.dll");
        if !dll_path.is_file() {
            bail!("LiteRT-LM runtime is missing: {}", dll_path.display());
        }
        if !model_path.is_file() {
            bail!("packaged model is missing: {}", model_path.display());
        }

        let api = Api::load(&dll_path)?;
        // Suppress verbose backend logs; errors still go to stderr.
        unsafe { (api.set_min_log_level)(3) };

        let requested_backend = env::var("YUUKEI_LITERT_BACKEND").ok();
        let backends: &[&str] = match requested_backend.as_deref() {
            Some("cpu") => &["cpu"],
            Some("gpu") => &["gpu"],
            _ => &["gpu", "cpu"],
        };
        let mut last_error = None;
        for &backend in backends {
            match create_engine(&api, &model_path, &cache_dir, backend) {
                Ok(engine) => {
                    return Ok(Self {
                        api,
                        engine,
                        backend,
                        model_name: "gemma-4-E4B-it",
                    });
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("no LiteRT-LM backend was attempted")))
    }

    pub fn metadata(&self) -> Value {
        json!({
            "provider": "litert-lm",
            "model": self.model_name,
            "backend": self.backend
        })
    }

    pub fn generate(
        &self,
        system_prompt: &str,
        prompt: &str,
        max_output_tokens: i32,
    ) -> Result<String> {
        let system_prompt = CString::new(system_prompt)?;
        let message = CString::new(serde_json::to_string(
            &json!({ "role": "user", "content": prompt }),
        )?)?;
        let extra_context = CString::new("{}")?;

        // SAFETY: All handles are created by this exact API instance, checked
        // for null, used serially on this process' main thread, and deleted in
        // reverse ownership order before returning.
        unsafe {
            let session_config = (self.api.session_config_create)();
            if session_config.is_null() {
                bail!("LiteRT-LM failed to create a session config");
            }
            (self.api.session_config_set_max_output_tokens)(session_config, max_output_tokens);

            let conversation_config = (self.api.conversation_config_create)();
            if conversation_config.is_null() {
                (self.api.session_config_delete)(session_config);
                bail!("LiteRT-LM failed to create a conversation config");
            }
            (self.api.conversation_config_set_session)(conversation_config, session_config);
            (self.api.session_config_delete)(session_config);
            (self.api.conversation_config_set_system)(conversation_config, system_prompt.as_ptr());

            let conversation = (self.api.conversation_create)(self.engine, conversation_config);
            (self.api.conversation_config_delete)(conversation_config);
            if conversation.is_null() {
                bail!("LiteRT-LM failed to create a conversation");
            }

            let optional_args = (self.api.conversation_optional_args_create)();
            if optional_args.is_null() {
                (self.api.conversation_delete)(conversation);
                bail!("LiteRT-LM failed to create conversation arguments");
            }
            (self.api.conversation_optional_args_set_max_output_tokens)(
                optional_args,
                max_output_tokens,
            );

            let response = (self.api.conversation_send_message)(
                conversation,
                message.as_ptr(),
                extra_context.as_ptr(),
                optional_args,
            );
            (self.api.conversation_optional_args_delete)(optional_args);
            if response.is_null() {
                (self.api.conversation_delete)(conversation);
                bail!("LiteRT-LM inference failed");
            }

            let response_text = (self.api.json_response_get_string)(response);
            let copied = if response_text.is_null() {
                Err(anyhow!("LiteRT-LM returned an empty response"))
            } else {
                CStr::from_ptr(response_text)
                    .to_str()
                    .map(str::to_owned)
                    .context("LiteRT-LM returned non-UTF-8 JSON")
            };
            (self.api.json_response_delete)(response);
            (self.api.conversation_delete)(conversation);
            extract_text(&copied?)
        }
    }
}

impl Drop for LiteRtEngine {
    fn drop(&mut self) {
        if !self.engine.is_null() {
            unsafe { (self.api.engine_delete)(self.engine) };
            self.engine = ptr::null_mut();
        }
    }
}

fn create_engine(
    api: &Api,
    model_path: &Path,
    cache_dir: &Path,
    backend: &'static str,
) -> Result<*mut c_void> {
    let model = CString::new(model_path.to_string_lossy().as_bytes())?;
    let cache_dir = CString::new(cache_dir.to_string_lossy().as_bytes())?;
    let backend_string = CString::new(backend)?;
    let settings = unsafe {
        (api.engine_settings_create)(
            model.as_ptr(),
            backend_string.as_ptr(),
            ptr::null(),
            ptr::null(),
        )
    };
    if settings.is_null() {
        bail!("LiteRT-LM rejected the {backend} engine settings");
    }
    unsafe { (api.engine_settings_set_max_tokens)(settings, 4096) };
    unsafe { (api.engine_settings_set_cache_dir)(settings, cache_dir.as_ptr()) };
    let engine = unsafe { (api.engine_create)(settings) };
    unsafe { (api.engine_settings_delete)(settings) };
    if engine.is_null() {
        bail!("LiteRT-LM could not initialize the {backend} backend");
    }
    Ok(engine)
}

fn extract_text(response: &str) -> Result<String> {
    let value: Value = serde_json::from_str(response).context("invalid LiteRT-LM response JSON")?;
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Ok(text.to_string());
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = content.as_str() {
            return Ok(text.to_string());
        }
        if let Some(parts) = content.as_array() {
            let text = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| part.as_str())
                })
                .collect::<String>();
            if !text.is_empty() {
                return Ok(text);
            }
        }
    }
    bail!("LiteRT-LM response did not contain text")
}

fn runtime_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("YUUKEI_LITERT_RUNTIME_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(env::current_exe()?
        .parent()
        .context("extension executable has no parent directory")?
        .to_path_buf())
}

fn model_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("YUUKEI_LITERT_MODEL_PATH") {
        return Ok(PathBuf::from(path));
    }
    let executable = env::current_exe()?;
    let bin_dir = executable
        .parent()
        .context("extension executable has no parent directory")?;
    let package_dir = bin_dir.parent().unwrap_or(bin_dir);
    Ok(package_dir.join("model").join("gemma-4-E4B-it.litertlm"))
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("YUUKEI_EXTENSION_DATA_DIR") {
        return Ok(PathBuf::from(path).join("litert-cache"));
    }
    Ok(env::temp_dir()
        .join("yuukei-intelligence")
        .join("litert-cache"))
}
