use rustix::{fs, io, net};

pub fn notify() -> io::Result<()> {
    // check if the system was loaded by systemd by verifying if the /run/systemd/system path
    // exists
    if fs::access("/run/systemd/system", fs::Access::EXISTS).is_ok() {
        let msg = b"READY=1";
        let Some(sock) = connect_notify_socket()? else {
            return Ok(());
        };

        let len = net::send(sock, msg, net::SendFlags::empty())?;
        if len != msg.len() {
            common::log::error!("failed to write full message to SystemD socket");
        }
    }
    Ok(())
}

#[cold]
fn connect_notify_socket() -> io::Result<Option<rustix::fd::OwnedFd>> {
    let socket_path = match unsafe { common::getenv(c"NOTIFY_SOCKET") } {
        Some(p) => p,
        None => return Ok(None),
    };

    let socket = net::socket_with(
        net::AddressFamily::UNIX,
        net::SocketType::DGRAM,
        net::SocketFlags::CLOEXEC,
        None,
    )?;

    let addr = net::SocketAddrUnix::new(socket_path)?;
    net::connect(&socket, &addr)?;
    Ok(Some(socket))
}
