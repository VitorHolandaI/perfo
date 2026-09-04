use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::Lang;
use crate::theme::Theme;

/// One row of help content.
///
/// `key` is the yellow key column (empty for non-key rows); `text` is the
/// description. Rows tagged `Colors` run their text through [`colorized`],
/// which understands `g:`/`y:`/`r:`/`a:` prefixes per `|`-separated segment.
pub struct HelpRow<'a> {
    pub kind: RowKind,
    pub key: &'a str,
    pub text: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Hdr,
    Key,
    Txt,
    Colors,
}

/// Renders rows: headers in accent+bold, keys in yellow, threshold words
/// in their semantic color, everything else in the theme foreground.
pub fn help_lines<'a>(rows: &'a [HelpRow<'a>], theme: &Theme) -> Vec<Line<'static>> {
    rows.iter()
        .map(|r| match r.kind {
            RowKind::Hdr => Line::from(Span::styled(
                format!("  {}", r.text),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            RowKind::Key => Line::from(vec![
                Span::styled(
                    format!("  {:<12}", r.key),
                    Style::default().fg(theme.yellow),
                ),
                Span::styled(r.text.to_string(), Style::default().fg(theme.fg)),
            ]),
            RowKind::Txt => Line::from(Span::styled(
                r.text.to_string(),
                Style::default().fg(theme.fg),
            )),
            RowKind::Colors => colorized(theme, r.text),
        })
        .collect()
}

/// "g:verde <5 | y:amarelo 5-10 | r:vermelho >10" -> colored spans; `a:` maps
/// to the accent (used for "blue" in the memory legend).
fn colorized(theme: &Theme, s: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for part in s.split('|') {
        let p = part.trim();
        let (col, txt) = match p.strip_prefix("g:") {
            Some(x) => (theme.green, x),
            None => match p.strip_prefix("y:") {
                Some(x) => (theme.yellow, x),
                None => match p.strip_prefix("r:") {
                    Some(x) => (theme.red, x),
                    None => match p.strip_prefix("a:") {
                        Some(x) => (theme.accent, x),
                        None => (theme.fg, p),
                    },
                },
            },
        };
        spans.push(Span::styled(txt.to_string(), Style::default().fg(col)));
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
    }
    Line::from(spans)
}

pub fn page(page: usize, lang: Lang, theme: &Theme) -> (String, Vec<Line<'static>>) {
    let (title, rows): (String, &[HelpRow<'static>]) = match (lang, page) {
        (Lang::Pt, 0) => (
            "help 1/5 — teclas".into(),
            &[
                hdr("PAINEIS — Tab alterna"),
                key("Tab", "alterna: Processos → CPU → IO → NET"),
                key("1-4", "focam o painel (1 CPU · 2 IO · 3 NET · 4 PROC);"),
                key("", "repetir o numero do painel em foco volta ao anterior"),
                key("← →", "no painel CPU: move o nucleo focado"),
                key("Enter", "filtra os processos do nucleo focado (Esc volta)"),
                key("↑ ↓", "selecionam processo; PgUp/PgDn/Home/End pulam"),
                txt(""),
                hdr("ACOES"),
                key("q", "sair · z pausa · s trace de syscalls"),
                key("k", "kill: 1=SIGTERM, 9=SIGKILL, 0=cancela (prompt)"),
                key("p/m", "ordena por CPU/MEM · i inverte a ordem"),
                key("c", "comando completo · t arvore · H threads · K kernel"),
                key("/", "busca · C tema de cor · L idioma PT/EN"),
                txt(""),
                hdr("HELP"),
                key("n/p", "proxima/anterior pagina (ou PgUp/PgDn)"),
                key("?/q", "fecha o help"),
            ],
        ),
        (Lang::En, 0) => (
            "help 1/5 — keys".into(),
            &[
                hdr("PAGES — Tab cycles"),
                key("Tab", "cycles: Cpu → IO → NET"),
                key("1-4", "focus a pane (1 CPU · 2 IO · 3 NET · 4 PROC);"),
                key("", "repeat the focused pane's number to return to previous"),
                key("← →", "CPU pane: move the focused core"),
                key("Enter", "filter processes of the focused core (Esc clears)"),
                key("↑ ↓", "select a process; PgUp/PgDn/Home/End jump"),
                txt(""),
                hdr("ACTIONS"),
                key("q", "quit · z pause · s syscall trace"),
                key("k", "kill: 1=SIGTERM, 9=SIGKILL, 0=cancel (prompt)"),
                key("p/m", "sort by CPU/MEM · i invert order"),
                key("c", "full command · t tree · H threads · K kernel"),
                key("/", "search · C color theme · L language PT/EN"),
                txt(""),
                hdr("HELP"),
                key("n/p", "next/previous page (or PgUp/PgDn)"),
                key("?/q", "close help"),
            ],
        ),
        (Lang::Pt, 1) => (
            "help 2/5 — bloco CPU".into(),
            &[
                hdr("VISAO GERAL"),
                key("overall", "uso total da CPU (barra + %)"),
                key("load a b c", "load average 1/5/15 min"),
                key("iowait", "% do tempo ocioso esperando I/O (gargalo de disco)"),
                txt(""),
                hdr("TIPOS DE NUCLEO"),
                cols("g:P  Performance: L2 privado, o mais rapido | y:E  Efficient: compartilha L2 num cluster | r:L  Low-power: clock baixo, economiza bateria"),
                txt(""),
                hdr("CORES E VALORES"),
                cols("GHz vs limite do nucleo: g:verde <33% | y:amarelo 33-66% | r:vermelho >66%"),
                cols("°C cor = tipo do nucleo (P/E/L) | barra: g:verde <50% | y:amarelo 50-80% | r:vermelho >80%"),
            ],
        ),
        (Lang::En, 1) => (
            "help 2/5 — CPU block".into(),
            &[
                hdr("OVERVIEW"),
                key("overall", "total CPU usage (bar + %)"),
                key("load a b c", "load average 1/5/15 min"),
                key("iowait", "% of idle time waiting on I/O (disk bottleneck)"),
                txt(""),
                hdr("CORE TYPES"),
                cols("g:P  Performance: private L2, fastest | y:E  Efficient: shares L2 in a cluster | r:L  Low-power: low clock, saves battery"),
                txt(""),
                hdr("COLORS AND VALUES"),
                cols("GHz vs core limit: g:green <33% | y:yellow 33-66% | r:red >66%"),
                cols("°C colored by core type (P/E/L) | bar: g:green <50% | y:yellow 50-80% | r:red >80%"),
            ],
        ),
        (Lang::Pt, 2) => (
            "help 3/5 — memoria e discos".into(),
            &[
                hdr("MEM — barra empilhada"),
                cols("g:verde = usado por apps (nao liberavel) | y:amarelo = cache (liberavel) | a:azul = buffers de I/O | cinza = livre"),
                txt("sem RAM, o kernel descarta o amarelo primeiro; o verde"),
                txt("so cai fechando apps — a soma de tudo = total"),
                key("swap", "uso com barra e %"),
                key("psi", "pressao de memoria (10s/60s/300s): estresse ANTES de travar"),
                cols("g:<5% ok | y:5-10% atencao | r:>10% estresse"),
                txt(""),
                hdr("DISKS"),
                txt("linha: nome | barra | % | usado/total | mount"),
                cols("barra: g:<70% ok | y:70-85% | r:>85%"),
                txt("subvolumes btrfs do mesmo disco agrupados num nome so"),
                txt(""),
                hdr("PAINEL IO (Tab)"),
                txt("taxas read/write desde o ultimo refresh; barras escalam"),
                txt("pelo disco mais rapido — leitura azul, escrita amarelo"),
                txt("pagina 5 explica cada stat e os bons valores"),
            ],
        ),
        (Lang::En, 2) => (
            "help 3/5 — memory and disks".into(),
            &[
                hdr("MEM — stacked bar"),
                cols("g:green = used by apps (not reclaimable) | y:yellow = cache (reclaimable) | a:blue = I/O buffers | gray = free"),
                txt("out of RAM the kernel drops the yellow first; green"),
                txt("only drops when apps close — segments sum to total"),
                key("swap", "usage with bar and %"),
                key("psi", "memory pressure (10s/60s/300s): stress BEFORE the freeze"),
                cols("g:<5% fine | y:5-10% watch | r:>10% stressed"),
                txt(""),
                hdr("DISKS"),
                txt("row: name | bar | % | used/total | mount"),
                cols("bar: g:<70% fine | y:70-85% | r:>85%"),
                txt("btrfs subvolumes of the same disk grouped under one name"),
                txt(""),
                hdr("IO PANE (Tab)"),
                txt("read/write rates since last refresh; bars scale to"),
                txt("the fastest disk — read blue, write yellow"),
                txt("page 5 explains every stat and good values"),
            ],
        ),
        (Lang::Pt, 3) => (
            "help 4/5 — processos e trace".into(),
            &[
                hdr("TABELA DE PROCESSOS"),
                txt("colunas: PID | USER | CPU% | MEM | COMMAND; seta = ordenada"),
                txt("CPU% e por nucleo (100% = 1 nucleo inteiro), como o htop"),
                key("Enter", "num nucleo = filtra os processos daquele nucleo"),
                key("H", "mostra threads · K mostra threads do kernel"),
                txt(""),
                hdr("TRACE (tecla s)"),
                txt("syscalls do processo selecionado ao vivo; s/q para parar"),
                txt("(detach — o processo continua rodando)"),
                txt("so funciona em filhos (yama ptrace_scope=1):"),
                txt("  perfo trace -- <comando>   → spawna e traca"),
                txt("  sudo sysctl kernel.yama.ptrace_scope=0  → libera todos"),
                txt(""),
                hdr("FORA DO TUI"),
                txt("perfo cpu --json  → dados pro widget do Omarchy"),
                txt("perfo bench       → perfil de refresh (flamegraph)"),
            ],
        ),
        (Lang::En, 3) => (
            "help 4/5 — processes and trace".into(),
            &[
                hdr("PROCESS TABLE"),
                txt("columns: PID | USER | CPU% | MEM | COMMAND; arrow = sorted"),
                txt("CPU% is per core (100% = one full core), like htop"),
                key("Enter", "on a core = filter processes of that core"),
                key("H", "show threads · K show kernel threads"),
                txt(""),
                hdr("TRACE (key s)"),
                txt("live syscalls of the selected process; s/q stops"),
                txt("(detach — the process keeps running)"),
                txt("only works on children (yama ptrace_scope=1):"),
                txt("  perfo trace -- <command>   → spawns and traces"),
                txt("  sudo sysctl kernel.yama.ptrace_scope=0  → allows all"),
                txt(""),
                hdr("OUTSIDE THE TUI"),
                txt("perfo cpu --json  → data for the Omarchy widget"),
                txt("perfo bench       → refresh profiling (flamegraph)"),
            ],
        ),
        (Lang::Pt, 4) => (
            "help 5/5 — IO: o que cada stat significa".into(),
            &[
                hdr("IOPS"),
                key("r/s w/s", "operacoes de leitura/escrita por segundo"),
                txt("milhares num NVMe; IOPS alto + taxa baixa = I/O aleatorio"),
                txt(""),
                hdr("LATENCIA — o numero que importa"),
                key("r_awt/w_awt", "tempo medio da op em ms, fila incluida"),
                cols("g:<2ms saudavel | y:2-10ms ok | r:>=10ms saturado — NVMe bom fica <1ms"),
                txt(""),
                hdr("FILA E BUSY"),
                key("fila", "requisicoes em voo (aqu-sz)"),
                cols("g:<2 | y:2-8 | r:>=8 — HDD: >1 ja enfileira"),
                key("busy", "% do tempo com alguma op em voo. CUIDADO: em NVMe"),
                txt("100% NAO e saturacao (16 filas paralelas);"),
                txt("confie em r_awt/w_awt + fila, nao em busy"),
                txt(""),
                hdr("MERGE E REQUEST SIZE (JSON)"),
                key("merge_pct", "% de requests juntados pelo kernel — alto = sequencial"),
                cols("req_kib: g:>=128KiB sequencial | y:4-16KiB randomico"),
                txt(""),
                hdr("PRESSAO E IOWAIT"),
                key("io pressure", "psi: % do tempo com I/O travando a maquina"),
                cols("g:<5 ok | y:5-10 atencao | r:>10 estresse"),
                key("iowait", "% ocioso esperando I/O (linha do CPU)"),
                txt(">15-20% sustentado + await alto = gargalo de disco;"),
                txt("iowait alto + await normal = paging (falta RAM)"),
                txt(""),
                hdr("I/O POR PROCESSO"),
                txt("read_bytes/write_bytes = bytes REAIS no storage (nao rchar/"),
                txt("wchar que incluem cache); so teus processos (yama),"),
                txt("root ve todos"),
            ],
        ),
        (Lang::En, 4) => (
            "help 5/5 — IO: what each stat means".into(),
            &[
                hdr("IOPS"),
                key("r/s w/s", "read/write operations per second"),
                txt("thousands on NVMe; high IOPS + low throughput = random I/O"),
                txt(""),
                hdr("LATENCY — the number that matters"),
                key("r_awt/w_awt", "average op time in ms, queue included"),
                cols("g:<2ms healthy | y:2-10ms ok | r:>=10ms saturated — healthy NVMe stays <1ms"),
                txt(""),
                hdr("QUEUE AND BUSY"),
                key("fila", "in-flight requests (aqu-sz)"),
                cols("g:<2 | y:2-8 | r:>=8 — HDD: >1 already queues"),
                key("busy", "% of time with an op in flight. CAUTION: on NVMe"),
                txt("100% is NOT saturation (16 parallel queues);"),
                txt("trust r_awt/w_awt + fila, not busy"),
                txt(""),
                hdr("MERGE AND REQUEST SIZE (JSON)"),
                key("merge_pct", "% of requests merged by the kernel — high = sequential"),
                cols("req_kib: g:>=128KiB sequential | y:4-16KiB random"),
                txt(""),
                hdr("PRESSURE AND IOWAIT"),
                key("io pressure", "psi: % of time I/O stalls the machine"),
                cols("g:<5 fine | y:5-10 watch | r:>10 stressed"),
                key("iowait", "% idle waiting on I/O (CPU line)"),
                txt("sustained >15-20% + high await = disk bottleneck;"),
                txt("high iowait + normal await = paging (low RAM)"),
                txt(""),
                hdr("PER-PROCESS I/O"),
                txt("read_bytes/write_bytes = real storage bytes (not rchar/"),
                txt("wchar which include cache); own processes only (yama),"),
                txt("root sees all"),
            ],
        ),
        (_, _) => unreachable!("help page out of range"),
    };
    (title, help_lines(rows, theme))
}

const fn hdr(text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Hdr,
        key: "",
        text,
    }
}

const fn key(key: &'static str, text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Key,
        key,
        text,
    }
}

const fn txt(text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Txt,
        key: "",
        text,
    }
}

const fn cols(text: &'static str) -> HelpRow<'static> {
    HelpRow {
        kind: RowKind::Colors,
        key: "",
        text,
    }
}
