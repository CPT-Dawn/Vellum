PREFIX ?= /usr
DESTDIR ?=
BINDIR := $(PREFIX)/bin
SYSTEMD_USER_DIR := $(PREFIX)/lib/systemd/user
LICENSE_DIR := $(PREFIX)/share/licenses/vellum

CARGO ?= cargo
TARGET_DIR ?= target/release

.PHONY: all build release install install-bins install-service install-license uninstall clean fmt clippy test

all: build

build:
	$(CARGO) build --release --workspace

release: build

install: build install-bins install-service install-license

install-bins:
	install -d "$(DESTDIR)$(BINDIR)"
	install -m755 "$(TARGET_DIR)/vellumd" "$(DESTDIR)$(BINDIR)/vellumd"
	install -m755 "$(TARGET_DIR)/vellum-tui" "$(DESTDIR)$(BINDIR)/vellum-tui"

install-service:
	install -d "$(DESTDIR)$(SYSTEMD_USER_DIR)"
	install -m644 "systemd/user/vellumd.service" "$(DESTDIR)$(SYSTEMD_USER_DIR)/vellumd.service"

install-license:
	install -d "$(DESTDIR)$(LICENSE_DIR)"
	install -m644 "LICENSE" "$(DESTDIR)$(LICENSE_DIR)/LICENSE"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/vellumd"
	rm -f "$(DESTDIR)$(BINDIR)/vellum-tui"
	rm -f "$(DESTDIR)$(SYSTEMD_USER_DIR)/vellumd.service"
	rm -f "$(DESTDIR)$(LICENSE_DIR)/LICENSE"

clean:
	$(CARGO) clean

fmt:
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

test:
	$(CARGO) test --workspace --all-targets
