use std::fs;
use std::io;
use std::thread;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
};
use sysinfo::{ProcessesToUpdate, System};

// ---------- Estado ----------
enum Orden {
    Cpu,
    Ram,
}

struct Estado {
    orden: Orden,
    seleccion: usize,
    detalle: Option<String>,
    pid_seguir: Option<String>,
}

struct MuestraCpu {
    total: u64,
    idle: u64,
}

struct Proceso {
    pid: String,
    nombre: String,
    cpu: f64,     // % de CPU (muestreado por sysinfo)
    memoria: u64, // bytes
}

fn fmt_bytes(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.0} MB", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{:.0} KB", b / 1024)
    }
}

// Color según el nivel de uso
fn color_nivel(pct: f64) -> Color {
    if pct < 60.0 {
        Color::Green
    } else if pct < 85.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

struct Datos {
    hostname: String,
    kernel: String,
    uptime: String,
    cpu: f64,
    ram_pct: f64,
    ram_texto: String,
    disco_pct: u16,
    disco_texto: String,
    procesos: Vec<Proceso>,
}

fn leer_muestra() -> MuestraCpu {
    let contenido = fs::read_to_string("/proc/stat").unwrap();
    let linea = contenido.lines().next().unwrap();
    let partes: Vec<&str> = linea.split_whitespace().collect();

    let mut total: u64 = 0;
    let mut idle: u64 = 0;
    for (i, parte) in partes.iter().enumerate().skip(1) {
        let valor: u64 = parte.parse().unwrap();
        total += valor;
        if i == 4 {
            idle = valor;
        }
    }
    MuestraCpu { total, idle }
}

fn recolectar(estado: &Estado, sys: &mut System) -> Datos {
    // ---- Etapa 2 + 7 ----
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_else(|_| String::from("desconocido"));
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| String::from("desconocido"));
    let uptime = fs::read_to_string("/proc/uptime").unwrap_or_else(|_| String::from("0"));
    let seconds: f64 = uptime
        .split_whitespace()
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0.0);
    let total_segs = seconds as u64;
    let uptime_fmt = format!(
        "{:02}:{:02}:{:02}",
        total_segs / 3600,
        (total_segs % 3600) / 60,
        total_segs % 60
    );

    // ---- Etapa 3 ----
    let a = leer_muestra();
    thread::sleep(Duration::from_millis(500));
    let b = leer_muestra();
    let delta_total = b.total - a.total;
    let delta_idle = b.idle - a.idle;
    let cpu = 100.0 * (1.0 - delta_idle as f64 / delta_total as f64);

    // ---- Etapa 4 ----
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap();
    let mut total_kb: u64 = 0;
    let mut disponible_kb: u64 = 0;
    for linea in meminfo.lines() {
        let partes: Vec<&str> = linea.split_whitespace().collect();
        if partes.len() >= 2 {
            let valor: u64 = partes[1].parse().unwrap_or(0);
            match partes[0] {
                "MemTotal:" => total_kb = valor,
                "MemAvailable:" => disponible_kb = valor,
                _ => {}
            }
        }
    }
    let usada_kb = total_kb - disponible_kb;
    let total_gb = total_kb as f64 / 1024.0 / 1024.0;
    let usada_gb = usada_kb as f64 / 1024.0 / 1024.0;
    let ram_pct = 100.0 * usada_kb as f64 / total_kb as f64;
    let ram_texto = format!("{:.1}/{:.1} GB", usada_gb, total_gb);

    // ---- Etapa 5 ----
    let salida = std::process::Command::new("df")
        .args(["-h", "/"])
        .output()
        .expect("no se pudo ejecutar df");
    let texto = String::from_utf8_lossy(&salida.stdout);
    let linea = texto.lines().nth(1).unwrap_or("");
    let partes: Vec<&str> = linea.split_whitespace().collect();
    let mut disco_pct: u16 = 0;
    let mut disco_texto = String::from("?");
    if partes.len() >= 5 {
        disco_pct = partes[4].trim_end_matches('%').parse().unwrap_or(0);
        disco_texto = format!("{} usados de {}", partes[2], partes[1]);
    }

    // ---- Etapa 8 + 12 + 16: procesos vía sysinfo, ordenados en Rust ----
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut procesos: Vec<Proceso> = sys
        .processes()
        .iter()
        .map(|(pid, p)| Proceso {
            pid: pid.to_string(),
            nombre: p.name().to_string_lossy().into_owned(),
            cpu: p.cpu_usage() as f64,
            memoria: p.memory(),
        })
        .collect();

    match estado.orden {
        Orden::Cpu => procesos.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        Orden::Ram => procesos.sort_by_key(|a| std::cmp::Reverse(a.memoria)),
    }
    procesos.truncate(25);

    Datos {
        hostname: hostname.trim().to_string(),
        kernel: kernel.trim().to_string(),
        uptime: uptime_fmt,
        cpu,
        ram_pct,
        ram_texto,
        disco_pct,
        disco_texto,
        procesos,
    }
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut estado = Estado {
        orden: Orden::Cpu,
        seleccion: 0,
        detalle: None,
        pid_seguir: None,
    };

    let mut sys = System::new();
    let mut ultima: Option<Datos> = None;

    

    (|| -> io::Result<()> {
        loop {
            // PID de la fila seleccionada en la lista ANTERIOR
            estado.pid_seguir = ultima
                .as_ref()
                .and_then(|prev| prev.procesos.get(estado.seleccion))
                .map(|p| p.pid.clone());

            let d = recolectar(&estado, &mut sys);

            if let Some(pid) = &estado.pid_seguir {
                if let Some(pos) = d.procesos.iter().position(|p| &p.pid == pid) {
                    estado.seleccion = pos;
                } else {
                    estado.seleccion = estado.seleccion.min(d.procesos.len().saturating_sub(1));
                }
            }
            ultima = Some(d);
            let d = ultima.as_ref().unwrap();

            // Ventana deslizante: mostramos 10 filas alrededor de la selección
            let inicio = estado.seleccion.saturating_sub(5);
            let inicio = inicio.min(d.procesos.len().saturating_sub(10));

            terminal.draw(|f| {
                // Terminal muy pequeña: aviso en vez de dibujo roto
                let area = f.area();
                if area.width < 60 || area.height < 16 {
                    let aviso = Paragraph::new("Terminal muy pequeña.\nAgranda la ventana.")
                        .block(Block::default().borders(Borders::ALL));
                    f.render_widget(aviso, area);
                    return;
                }

                // ---- Etapa 14: vista de detalle ----
                if let Some(pid) = &estado.detalle {
                    let salida = std::process::Command::new("ps")
                        .args([
                            "-p",
                            pid,
                            "-o",
                            "pid,user,comm,%cpu,%mem,etime,args",
                            "--no-headers",
                        ])
                        .output()
                        .expect("no se pudo ejecutar ps");
                    let info_ps = String::from_utf8_lossy(&salida.stdout);
                    let texto = format!(
                        "Información detallada del proceso {}\n\n{}\n\nPulsa Esc para volver.",
                        pid, info_ps
                    );
                    let detalle = Paragraph::new(texto).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" DETALLE (Esc=volver, q=salir) "),
                    );
                    f.render_widget(detalle, f.area());
                } else {
                    let franjas = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(5),
                            Constraint::Length(3),
                            Constraint::Length(3),
                            Constraint::Length(3),
                            Constraint::Min(5),
                        ])
                        .split(f.area());

                    let info = Paragraph::new(format!(
                        "Hostname: {}\nKernel:   {}\nUPTIME:   {}",
                        d.hostname, d.kernel, d.uptime
                    ))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" SYSTEM MONITOR "),
                    );
                    f.render_widget(info, franjas[0]);

                    let cpu = Gauge::default()
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!("CPU  {:.0}%", d.cpu)),
                        )
                        .gauge_style(Style::default().fg(color_nivel(d.cpu)))
                        .percent(d.cpu.clamp(0.0, 100.0) as u16)
                        .label(format!("{:.1}%", d.cpu));
                    f.render_widget(cpu, franjas[1]);

                    let ram = Gauge::default()
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!("RAM  {}", d.ram_texto)),
                        )
                        .gauge_style(Style::default().fg(color_nivel(d.ram_pct)))
                        .percent(d.ram_pct.clamp(0.0, 100.0) as u16)
                        .label(format!("{:.0}%", d.ram_pct));
                    f.render_widget(ram, franjas[2]);

                    let disco = Gauge::default()
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!("DISCO  {}", d.disco_texto)),
                        )
                        .gauge_style(Style::default().fg(color_nivel(d.disco_pct as f64)))
                        .percent(d.disco_pct)
                        .label(format!("{}%", d.disco_pct));
                    f.render_widget(disco, franjas[3]);

                    // Tabla de procesos con selección resaltada
                    let titulo = match estado.orden {
                        Orden::Cpu => "PROCESOS (CPU)  [c]=RAM [q]=salir",
                        Orden::Ram => "PROCESOS (RAM)  [c]=CPU [q]=salir",
                    };
                    let mut lineas: Vec<Line> = Vec::new();
                    lineas.push(Line::from(Span::styled(
                        format!("{:<8} {:<20} {:>6} {:>10}", "PID", "PROCESO", "CPU", "RAM"),
                        Style::default().add_modifier(Modifier::BOLD),
                    )));
                    for (i, p) in d.procesos.iter().enumerate().skip(inicio).take(10) {
                        let abs = i + inicio;
                        let texto = format!(
                            "{:<8} {:<20} {:>6} {:>10}",
                            p.pid,
                            p.nombre,
                            format!("{:.1}%", p.cpu),
                            fmt_bytes(p.memoria)
                        );
                        if abs == estado.seleccion {
                            lineas.push(Line::from(Span::styled(
                                texto,
                                Style::default().fg(Color::Black).bg(Color::White),
                            )));
                        } else {
                            lineas.push(Line::from(texto));
                        }
                    }
                    let tabla = Paragraph::new(lineas)
                        .block(Block::default().borders(Borders::ALL).title(titulo));
                    f.render_widget(tabla, franjas[4]);
                }
            })?;

            // ---- Etapa 13: manejo de teclas ----
            if event::poll(Duration::from_millis(1000))?
                && let Event::Key(tecla) = event::read()?
                    && tecla.kind == KeyEventKind::Press {
                        // Ctrl+C sale siempre, estés en la vista que estés
                        if tecla.code == KeyCode::Char('c')
                            && tecla.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            break;
                        }
                        // En modo detalle: solo Esc o q
                        if estado.detalle.is_some() {
                            match tecla.code {
                                KeyCode::Char('q') => break,
                                KeyCode::Esc => estado.detalle = None,
                                _ => {}
                            }
                            continue;
                        }
                        match tecla.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Enter => {
                                // Guardar el PID del proceso seleccionado
                                if let Some(p) = d.procesos.get(estado.seleccion) {
                                    estado.detalle = Some(p.pid.clone());
                                }
                            }
                            KeyCode::Char('c') => {
                                estado.orden = match estado.orden {
                                    Orden::Cpu => Orden::Ram,
                                    Orden::Ram => Orden::Cpu,
                                };
                                estado.seleccion = 0;
                                // Olvidar el PID seguido al cambiar de orden
                                if let Some(prev) = &mut ultima {
                                    prev.procesos.clear();
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                estado.seleccion = estado.seleccion.saturating_sub(1);
                            }
                            KeyCode::Down | KeyCode::Char('j')
                                if estado.seleccion + 1 < d.procesos.len() => {
                                    estado.seleccion += 1;
                                }
                            _ => {}
                        }
                    }
        }

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        Ok(())
    })()
}
