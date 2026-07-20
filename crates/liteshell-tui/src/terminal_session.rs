use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
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
        if let Err(e) = execute!(out, EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(e);
        }
        let terminal = match Terminal::new(CrosstermBackend::new(out)) {
            Ok(v) => v,
            Err(e) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
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
        let raw = disable_raw_mode();
        let screen = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        self.active = false;
        raw.and(screen)
    }
    fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
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
