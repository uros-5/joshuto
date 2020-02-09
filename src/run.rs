use std::process;
use std::time;

use termion::event::Key;

use crate::commands::{CommandKeybind, FileOperationThread, JoshutoCommand, ReloadDirList};
use crate::config::{self, JoshutoCommandMapping, JoshutoConfig};
use crate::context::JoshutoContext;
use crate::tab::JoshutoTab;
use crate::ui;
use crate::util::event::{Event, Events};
use crate::window::JoshutoPanel;
use crate::window::JoshutoView;

fn recurse_get_keycommand(keymap: &JoshutoCommandMapping) -> Option<&JoshutoCommand> {
    let (term_rows, term_cols) = ui::getmaxyx();
    ncurses::timeout(-1);

    let events = Events::new();
    let event = {
        let keymap_len = keymap.len();
        let win = JoshutoPanel::new(
            keymap_len as i32 + 1,
            term_cols,
            ((term_rows - keymap_len as i32 - 2) as usize, 0),
        );

        let mut display_vec: Vec<String> = keymap
            .iter()
            .map(|(k, v)| format!("  {:?}\t{}", k, v))
            .collect();
        display_vec.sort();

        win.move_to_top();
        ui::display_menu(&win, &display_vec);
        ncurses::doupdate();

        events.next()
    };
    ncurses::doupdate();

    match event {
        Ok(Event::Input(input)) => match input {
            Key::Esc => {
                None
            }
            key @ Key::Char(_) => {
                match keymap.get(&key) {
                    Some(CommandKeybind::CompositeKeybind(m)) => recurse_get_keycommand(&m),
                    Some(CommandKeybind::SimpleKeybind(s)) => Some(s.as_ref()),
                    _ => None,
                }
            }
            _ => {
                None
            }
        }
        _ => None,
    }
}

fn reload_tab(
    index: usize,
    context: &mut JoshutoContext,
    view: &JoshutoView,
) -> std::io::Result<()> {
    ReloadDirList::reload(index, context)?;
    if index == context.curr_tab_index {
        let dirty_tab = &mut context.tabs[index];
        dirty_tab.refresh(view, &context.config_t);
    }
    Ok(())
}

fn join_thread(
    context: &mut JoshutoContext,
    thread: FileOperationThread<u64, fs_extra::TransitProcess>,
    view: &JoshutoView,
) -> std::io::Result<()> {
    ncurses::werase(view.bot_win.win);
    ncurses::doupdate();

    let (tab_src, tab_dest) = (thread.tab_src, thread.tab_dest);
    match thread.handle.join() {
        Err(e) => {
            ui::wprint_err(&view.bot_win, format!("{:?}", e).as_str());
            view.bot_win.queue_for_refresh();
        }
        Ok(_) => {
            if tab_src < context.tabs.len() {
                reload_tab(tab_src, context, view)?;
            }
            if tab_dest != tab_src && tab_dest < context.tabs.len() {
                reload_tab(tab_dest, context, view)?;
            }
        }
    }
    Ok(())
}

fn process_threads(context: &mut JoshutoContext, view: &JoshutoView) -> std::io::Result<()> {
    let thread_wait_duration: time::Duration = time::Duration::from_millis(100);
    for i in 0..context.threads.len() {
        match &context.threads[i].recv_timeout(&thread_wait_duration) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let thread = context.threads.swap_remove(i);
                join_thread(context, thread, view)?;
                ncurses::doupdate();
                break;
            }
            Ok(progress_info) => {
                ui::draw_fs_operation_progress(&view.bot_win, &progress_info);
                ncurses::doupdate();
            }
            _ => {}
        }
    }
    Ok(())
}

#[inline]
fn resize_handler(context: &mut JoshutoContext, view: &JoshutoView) {
    ui::redraw_tab_view(&view.tab_win, &context);

    let curr_tab = &mut context.tabs[context.curr_tab_index];
    curr_tab.refresh(view, &context.config_t);
    ncurses::doupdate();
}

fn init_context(context: &mut JoshutoContext, view: &JoshutoView) {
    match std::env::current_dir() {
        Ok(curr_path) => match JoshutoTab::new(curr_path, &context.config_t.sort_option) {
            Ok(tab) => {
                context.tabs.push(tab);
                context.curr_tab_index = context.tabs.len() - 1;

                ui::redraw_tab_view(&view.tab_win, &context);
                let curr_tab = &mut context.tabs[context.curr_tab_index];
                curr_tab.refresh(view, &context.config_t);
                ncurses::doupdate();
            }
            Err(e) => {
                ui::end_ncurses();
                eprintln!("{}", e);
                process::exit(1);
            }
        },
        Err(e) => {
            ui::end_ncurses();
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

pub fn run(config_t: JoshutoConfig, keymap_t: JoshutoCommandMapping) {
    ui::init_ncurses();

    let mut context = JoshutoContext::new(config_t);
    let mut view = JoshutoView::new(context.config_t.column_ratio);
    init_context(&mut context, &view);

    let events = Events::new();
    while !context.exit {
        let event = events.next();
        if let Ok(event) = event {
            match event {
                Event::Input(key) => {
                    let keycommand = match keymap_t.get(&key) {
                        Some(CommandKeybind::CompositeKeybind(m)) => match recurse_get_keycommand(&m) {
                            Some(s) => s,
                            None => {
                                ui::wprint_err(&view.bot_win, &format!("Unknown keycode: {:?}", key));
                                ncurses::doupdate();
                                continue;
                            }
                        },
                        Some(CommandKeybind::SimpleKeybind(s)) => {
                            s.as_ref()
                        }
                        None => {
                            ui::wprint_err(&view.bot_win, &format!("Unknown keycode: {:?}", key));
                            ncurses::doupdate();
                            continue;
                        }
                    };
                    match keycommand.execute(&mut context, &view) {
                        Err(e) => {
                            ui::wprint_err(&view.bot_win, e.cause());
                        }
                        _ => {}
                    }
                    ncurses::doupdate();
                }
                event => ui::wprint_err(&view.bot_win, &format!("Unknown keycode: {:?}", event)),
            }
            ncurses::doupdate();
        }
    }
    ui::end_ncurses();
}
