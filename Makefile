PREFIX ?= $(HOME)/.local
APP_ID := io.github.omaralzand.Prime95Gtk

build:
	cargo build --release

install: build
	install -Dm755 target/release/prime95-gtk $(PREFIX)/bin/prime95-gtk
	install -Dm644 data/$(APP_ID).desktop $(PREFIX)/share/applications/$(APP_ID).desktop
	install -Dm644 data/icons/$(APP_ID).svg $(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	-update-desktop-database $(PREFIX)/share/applications

uninstall:
	rm -f $(PREFIX)/bin/prime95-gtk $(PREFIX)/share/applications/$(APP_ID).desktop \
	      $(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg

.PHONY: build install uninstall
