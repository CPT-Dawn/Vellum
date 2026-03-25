use crate::wayland::zwlr_layer_shell_v1::Layer;
use common::ipc::PixelFormat;
use common::log;
use core::ffi::CStr;
use std::borrow::Cow;

pub struct Cli {
    pub format: Option<PixelFormat>,
    pub quiet: bool,
    pub no_cache: bool,
    pub layer: Layer,
    pub namespace: String,
}

impl Cli {
    pub fn new(args: &[*const core::ffi::c_char]) -> Result<Option<Self>, CliError> {
        let mut quiet = false;
        let mut no_cache = false;
        let mut format = None;
        let mut layer = Layer::background;
        let mut namespace = String::new();
        let mut args = args
            .iter()
            .map(|arg| unsafe { CStr::from_ptr(*arg) }.to_bytes());
        args.next(); // skip the first argument

        while let Some(arg) = args.next() {
            match arg {
                b"-f" | b"--format" => match args.next() {
                    Some(b"argb") => format = Some(PixelFormat::Argb),
                    Some(b"xrgb") => {
                        log::warn!(
                            "xrgb is deprecated. Use `--format argb` instead.\n\
                            \tNote this is the default, so you can also just omit it."
                        );
                        format = Some(PixelFormat::Argb);
                    }
                    Some(b"abgr") => format = Some(PixelFormat::Abgr),
                    Some(b"rgb") => format = Some(PixelFormat::Rgb),
                    Some(b"bgr") => format = Some(PixelFormat::Bgr),
                    None => return Err(CliError::AbsentFormat),
                    Some(other) => {
                        return Err(CliError::UnrecognizedFormat(String::from_utf8_lossy(other)));
                    }
                },
                b"-l" | b"--layer" => match args.next() {
                    Some(b"background") => layer = Layer::background,
                    Some(b"bottom") => layer = Layer::bottom,
                    None => return Err(CliError::AbsentLayer),
                    Some(other) => {
                        return Err(CliError::UnrecognizedLayer(String::from_utf8_lossy(other)));
                    }
                },
                b"-n" | b"--namespace" => {
                    namespace = match args.next() {
                        Some(s) => String::from_utf8_lossy(s).to_string(),
                        None => return Err(CliError::AbsentNamespace),
                    }
                }
                b"--no-cache" => no_cache = true,
                b"-q" | b"--quiet" => quiet = true,
                b"-h" | b"--help" => {
                    let msg = b"\
vellum-daemon

Options:

    -f|--format <argb|abgr|rgb|bgr>
        Force the use of a specific wl_shm format.

        By default, vellum-daemon will use argb, because it is most widely
        supported. Generally speaking, formats with 3 channels will use 3/4 the
        memory of formats with 4 channels. Also, bgr formats are more efficient
        than rgb formats because we do not need to do an extra swap of the bytes
        when decoding the image (though the difference is unnoticiable).

    -l|--layer <background|bottom>
        Which layer to display the background in. Defaults to `background`.

        We do not accept layers `top` and `overlay` because those would make
        your desktop unusable by simply putting an image on top of everything
        else. If there is ever a use case for these, we can reconsider this.

    -n|--namespace <namespace>
        Which wayland namespace to append to `vellum-daemon`.

        The resulting namespace will the `vellum-daemon<specified namespace>`.
        This also affects the name of the `vellum-daemon` socket we will use to
        communicate with the `client`. Specifically, our socket name is
        ${{WAYLAND_DISPLAY}}-vellum-daemon.<specified namespace>.socket.

        Some compositors can have several different wallpapers per output. This
        allows you to differentiate between them. Most users will probably not have
        to set anything in this option.

    --no-cache
        Don't search the cache for the last wallpaper for each output.
        Useful if you always want to select which image 'vellum' loads manually
        from the TUI.

    -q|--quiet    will only log errors
    -h|--help     print help
    -V|--version  print version\n";
                    let stdout = unsafe { rustix::stdio::stdout() };
                    _ = rustix::io::write(stdout, msg);
                    return Ok(None);
                }
                b"-V" | b"--version" => {
                    let stdout = unsafe { rustix::stdio::stdout() };
                    let bufs = [
                        rustix::io::IoSlice::new(b"vellum-daemon "),
                        rustix::io::IoSlice::new(env!("CARGO_PKG_VERSION").as_bytes()),
                        rustix::io::IoSlice::new(b"\n"),
                    ];
                    _ = rustix::io::writev(stdout, &bufs);
                    return Ok(None);
                }
                other => {
                    return Err(CliError::UnrecognizedArgument(String::from_utf8_lossy(
                        other,
                    )));
                }
            }
        }

        Ok(Some(Self {
            format,
            quiet,
            no_cache,
            layer,
            namespace,
        }))
    }
}

#[derive(Debug)]
pub enum CliError {
    AbsentFormat,
    UnrecognizedFormat(Cow<'static, str>),
    AbsentLayer,
    UnrecognizedLayer(Cow<'static, str>),
    AbsentNamespace,
    UnrecognizedArgument(Cow<'static, str>),
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CliError::AbsentFormat => f.write_str("format was not provided"),
            CliError::UnrecognizedFormat(format) => f.write_fmt(format_args!(
                "`--format` command line option must be one of: 'argb', 'abgr', 'rgb' or 'bgr'\n\
                Found: '{format}'"
            )),
            CliError::AbsentLayer => f.write_str("layer was not provided"),
            CliError::UnrecognizedLayer(layer) => f.write_fmt(format_args!(
                "`--layer` command line option must be one of: 'background', 'bottom'\n\
                Found: '{layer}'"
            )),
            CliError::AbsentNamespace => f.write_str("namespace was not provided"),
            CliError::UnrecognizedArgument(arg) => f.write_fmt(format_args!(
                "Unrecognized command line argument: {arg}\n\
                Run -h|--help to know what arguments are recognized!",
            )),
        }
    }
}

impl core::error::Error for CliError {}
