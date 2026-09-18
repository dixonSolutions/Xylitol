PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo
APP_ID := dev.xylitol.Xylitol

.PHONY: all build check test fmt clippy install uninstall clean

all: build

build:
	$(CARGO) build --release

check: fmt clippy test

test:
	$(CARGO) test --workspace

fmt:
	$(CARGO) fmt --all -- --check

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

install: build
	install -Dm755 target/release/xylitol      $(DESTDIR)$(PREFIX)/bin/xylitol
	install -Dm755 target/release/xylitol-cli  $(DESTDIR)$(PREFIX)/bin/xylitol-cli
	install -Dm644 data/$(APP_ID).desktop      $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	install -Dm644 data/$(APP_ID).metainfo.xml $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	install -Dm644 data/icons/hicolor/scalable/apps/$(APP_ID).svg \
	    $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/xylitol
	rm -f $(DESTDIR)$(PREFIX)/bin/xylitol-cli
	rm -f $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	rm -f $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg

clean:
	$(CARGO) clean
