use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};
pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;
pub struct TerminalSession {
    terminal: Option<TuiTerminal>,
    active: bool,
}
impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        if let Err(error) = execute!(out, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        let terminal = match Terminal::new(CrosstermBackend::new(out)) {
            Ok(v) => v,
            Err(e) => {
                let mut out = io::stdout();
                let _ = execute!(out, DisableMouseCapture);
                let _ = disable_raw_mode();
                return Err(e);
            }
        };
        Ok(Self {
            terminal: Some(terminal),
            active: true,
        })
    }
    pub fn terminal(&mut self) -> &mut TuiTerminal {
        self.terminal.as_mut().expect("terminal")
    }
    fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        let mut out = io::stdout();
        let mouse_result = execute!(out, DisableMouseCapture);
        let raw_result = disable_raw_mode();
        self.active = false;
        mouse_result.and(raw_result)
    }
    fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        enable_raw_mode()?;
        let mut out = io::stdout();
        if let Err(error) = execute!(out, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        self.active = true;
        self.terminal.as_mut().unwrap().clear()?;
        Ok(())
    }
    pub fn suspend<T>(&mut self, f: impl FnOnce() -> T) -> io::Result<T> {
        self.leave()?;
        struct Restore(*mut TerminalSession);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    let _ = (*self.0).resume();
                }
            }
        }
        let guard = Restore(self);
        let result = f();
        drop(guard);
        Ok(result)
    }
}
impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}
