use axum::{
    extract::{Form, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use axum_server::tls_rustls::RustlsConfig;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
};

const ARQUIVO_DADOS: &str = "boletim.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Disciplina {
    nome: String,
    notas: Vec<Option<f64>>,
    aulas_totais: u32,
    faltas: u32,
}

impl Disciplina {
    fn nova(nome: String, qtd_avaliacoes: usize, aulas_totais: u32) -> Self {
        Self {
            nome,
            notas: vec![None; qtd_avaliacoes.max(1)],
            aulas_totais: aulas_totais.max(1),
            faltas: 0,
        }
    }

    fn media_atual(&self) -> f64 {
        let lancadas: Vec<f64> = self.notas.iter().filter_map(|&n| n).collect();
        if lancadas.is_empty() {
            return 0.0;
        }
        lancadas.iter().sum::<f64>() / (lancadas.len() as f64)
    }

    fn frequencia_percentual(&self) -> f64 {
        let presencas = self.aulas_totais.saturating_sub(self.faltas);
        (presencas as f64 / self.aulas_totais as f64) * 100.0
    }

    fn quanto_falta(&self, alvo: f64) -> Option<f64> {
        let total = self.notas.len() as f64;
        let pendentes = self.notas.iter().filter(|n| n.is_none()).count() as f64;
        if pendentes == 0.0 {
            return None;
        }
        let soma_atual: f64 = self.notas.iter().filter_map(|&n| n).sum();
        let necessario = (alvo * total) - soma_atual;
        Some((necessario / pendentes).max(0.0))
    }

    fn status(&self) -> (&'static str, &'static str) {
        if self.frequencia_percentual() < 75.0 {
            return ("Reprovado por Falta", "badge-red");
        }
        let todas_lancadas = self.notas.iter().all(|n| n.is_some());
        let media = self.media_atual();

        if !todas_lancadas {
            return ("Cursando", "badge-blue");
        }
        if media >= 7.0 {
            ("Aprovado", "badge-green")
        } else if media >= 5.0 {
            ("Recuperação", "badge-yellow")
        } else {
            ("Reprovado por Nota", "badge-red")
        }
    }
}

#[derive(Serialize, Deserialize)]
struct EstadoBoletim {
    aluno: String,
    disciplinas: HashMap<String, Disciplina>,
}

impl EstadoBoletim {
    fn carregar() -> Self {
        if let Ok(conteudo) = fs::read_to_string(ARQUIVO_DADOS) {
            if let Ok(dados) = serde_json::from_str(&conteudo) {
                println!("📂 Dados carregados de '{}'", ARQUIVO_DADOS);
                return dados;
            }
        }
        Self {
            aluno: "Kauê".to_string(),
            disciplinas: HashMap::new(),
        }
    }

    fn salvar(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            if let Err(e) = fs::write(ARQUIVO_DADOS, json) {
                eprintln!("Erro ao salvar {}: {}", ARQUIVO_DADOS, e);
            }
        }
    }
}

type AppState = Arc<RwLock<EstadoBoletim>>;

#[derive(Deserialize)]
struct AlunoForm {
    aluno: String,
}

#[derive(Deserialize)]
struct NovaDisciplinaForm {
    nome: String,
    qtd_avaliacoes: usize,
    aulas_totais: u32,
}

#[derive(Deserialize)]
struct LancarNotaForm {
    disciplina: String,
    avaliacao: usize,
    nota: String,
}

#[derive(Deserialize)]
struct GerenciarFaltaForm {
    disciplina: String,
    modo: String,
    faltas: u32,
}

#[derive(Deserialize)]
struct DisciplinaUnicaForm {
    disciplina: String,
}

#[tokio::main]
async fn main() {
    let estado = Arc::new(RwLock::new(EstadoBoletim::carregar()));

    let app = Router::new()
        .route("/", get(pagina_inicial))
        .route("/exportar-csv", get(exportar_csv))
        .route("/aluno", post(atualizar_aluno))
        .route("/disciplina", post(adicionar_disciplina))
        .route("/nota", post(lancar_nota))
        .route("/falta", post(gerenciar_falta))
        .route("/zerar-faltas", post(zerar_faltas))
        .route("/remover", post(remover_disciplina))
        .with_state(estado);

    let config = RustlsConfig::from_pem_file(
        PathBuf::from("boletim.unip.pem"),
        PathBuf::from("boletim.unip-key.pem"),
    )
    .await
    .expect("Certificados boletim.unip.pem e boletim.unip-key.pem não encontrados na pasta!");

    let addr = SocketAddr::from(([0, 0, 0, 0], 443));
    println!("🔒 Boletim UNIP Seguro rodando em: https://boletim.unip");

    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn exportar_csv(State(state): State<AppState>) -> impl IntoResponse {
    let dados = state.read().unwrap();
    let mut lista: Vec<&Disciplina> = dados.disciplinas.values().collect();
    lista.sort_by(|a, b| a.nome.cmp(&b.nome));

    let mut csv = String::from("\u{FEFF}Aluno;Disciplina;Avaliacoes;Media;Aulas Totais;Faltas;Frequencia (%);Situacao\n");

    for d in &lista {
        let notas_str: Vec<String> = d
            .notas
            .iter()
            .enumerate()
            .map(|(i, n)| match n {
                Some(v) => format!("Av{}: {:.1}", i + 1, v),
                None => format!("Av{}: --", i + 1),
            })
            .collect();

        let (status_txt, _) = d.status();
        let linha = format!(
            "{};{};{};{:.2};{};{};{:.1}%;{}\n",
            dados.aluno,
            d.nome,
            notas_str.join(" | "),
            d.media_atual(),
            d.aulas_totais,
            d.faltas,
            d.frequencia_percentual(),
            status_txt
        );
        csv.push_str(&linha.replace('.', ","));
    }

    let cr = if lista.is_empty() {
        0.0
    } else {
        lista.iter().map(|d| d.media_atual()).sum::<f64>() / (lista.len() as f64)
    };
    csv.push_str(&format!("\nMedia Geral (CR);{:.2}\n", cr).replace('.', ","));

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"boletim.csv\"",
            ),
        ],
        csv,
    )
}

async fn atualizar_aluno(State(state): State<AppState>, Form(form): Form<AlunoForm>) -> Redirect {
    if let Ok(mut dados) = state.write() {
        let nome = form.aluno.trim();
        if !nome.is_empty() {
            dados.aluno = nome.to_string();
            dados.salvar();
        }
    }
    Redirect::to("/")
}

async fn adicionar_disciplina(
    State(state): State<AppState>,
    Form(form): Form<NovaDisciplinaForm>,
) -> Redirect {
    if let Ok(mut dados) = state.write() {
        let nome = form.nome.trim();
        if !nome.is_empty() {
            let chave = nome.to_lowercase();
            dados.disciplinas.insert(
                chave,
                Disciplina::nova(nome.to_string(), form.qtd_avaliacoes, form.aulas_totais),
            );
            dados.salvar();
        }
    }
    Redirect::to("/")
}

async fn lancar_nota(State(state): State<AppState>, Form(form): Form<LancarNotaForm>) -> Redirect {
    if let Ok(mut dados) = state.write() {
        if let Some(disc) = dados.disciplinas.get_mut(&form.disciplina.to_lowercase()) {
            if let Ok(valor) = form.nota.replace(',', ".").trim().parse::<f64>() {
                if (0.0..=10.0).contains(&valor) && form.avaliacao >= 1 && form.avaliacao <= disc.notas.len() {
                    disc.notas[form.avaliacao - 1] = Some(valor);
                    dados.salvar();
                }
            }
        }
    }
    Redirect::to("/")
}

async fn gerenciar_falta(
    State(state): State<AppState>,
    Form(form): Form<GerenciarFaltaForm>,
) -> Redirect {
    if let Ok(mut dados) = state.write() {
        if let Some(disc) = dados.disciplinas.get_mut(&form.disciplina.to_lowercase()) {
            match form.modo.as_str() {
                "definir" => {
                    if form.faltas <= disc.aulas_totais {
                        disc.faltas = form.faltas;
                        dados.salvar();
                    }
                }
                _ => {
                    if disc.faltas + form.faltas <= disc.aulas_totais {
                        disc.faltas += form.faltas;
                        dados.salvar();
                    }
                }
            }
        }
    }
    Redirect::to("/")
}

async fn zerar_faltas(
    State(state): State<AppState>,
    Form(form): Form<DisciplinaUnicaForm>,
) -> Redirect {
    if let Ok(mut dados) = state.write() {
        if let Some(disc) = dados.disciplinas.get_mut(&form.disciplina.to_lowercase()) {
            disc.faltas = 0;
            dados.salvar();
        }
    }
    Redirect::to("/")
}

async fn remover_disciplina(
    State(state): State<AppState>,
    Form(form): Form<DisciplinaUnicaForm>,
) -> Redirect {
    if let Ok(mut dados) = state.write() {
        if dados.disciplinas.remove(&form.disciplina.to_lowercase()).is_some() {
            dados.salvar();
        }
    }
    Redirect::to("/")
}

async fn pagina_inicial(State(state): State<AppState>) -> Html<String> {
    let dados = state.read().unwrap();
    let mut lista: Vec<&Disciplina> = dados.disciplinas.values().collect();
    lista.sort_by(|a, b| a.nome.cmp(&b.nome));

    let cr = if lista.is_empty() {
        0.0
    } else {
        lista.iter().map(|d| d.media_atual()).sum::<f64>() / (lista.len() as f64)
    };

    let mut opcoes_select = String::new();
    let mut linhas_tabela = String::new();

    for d in &lista {
        opcoes_select.push_str(&format!(
            "<option value=\"{}\">{} ({} av. | {} faltas)</option>",
            d.nome,
            d.nome,
            d.notas.len(),
            d.faltas
        ));

        let notas_html: String = d
            .notas
            .iter()
            .enumerate()
            .map(|(i, n)| match n {
                Some(v) => format!("<span class='nota-chip'>Av{}: <b>{:.1}</b></span>", i + 1, v),
                None => format!("<span class='nota-chip vazio'>Av{}: --</span>", i + 1),
            })
            .collect::<Vec<_>>()
            .join(" ");

        let (status_txt, badge_class) = d.status();
        let meta_txt = match d.quanto_falta(7.0) {
            Some(nec) if nec > 10.0 => format!("Precisa {:.1}/av (Impossível)", nec),
            Some(nec) if nec > 0.0 => format!("Precisa {:.1} nas restantes", nec),
            Some(_) => "Meta 7.0 atingida!".to_string(),
            None => "Concluído".to_string(),
        };

        let btn_zerar = if d.faltas > 0 {
            format!(
                r#"<form action="/zerar-faltas" method="post" style="display:inline; margin-left:6px;">
                    <input type="hidden" name="disciplina" value="{}">
                    <button type="submit" class="btn-reset" title="Zerar faltas desta disciplina">Zerar</button>
                </form>"#,
                d.nome
            )
        } else {
            String::new()
        };

        linhas_tabela.push_str(&format!(
            r#"
            <tr>
                <td><strong>{}</strong></td>
                <td>{}</td>
                <td><strong>{:.2}</strong></td>
                <td>
                    {:.1}% <small>({}/{})</small>
                    <br><small class="meta">Faltas: <b>{}</b></small>{}
                </td>
                <td><span class="badge {}">{}</span><br><small class="meta">{}</small></td>
                <td>
                    <form action="/remover" method="post" style="margin:0;">
                        <input type="hidden" name="disciplina" value="{}">
                        <button type="submit" class="btn-del" title="Remover disciplina">✕</button>
                    </form>
                </td>
            </tr>
            "#,
            d.nome,
            notas_html,
            d.media_atual(),
            d.frequencia_percentual(),
            d.aulas_totais - d.faltas,
            d.aulas_totais,
            d.faltas,
            btn_zerar,
            badge_class,
            status_txt,
            meta_txt,
            d.nome
        ));
    }

    if lista.is_empty() {
        linhas_tabela = r#"<tr><td colspan="6" style="text-align:center; color:#94a3b8; padding:2rem;">Nenhuma disciplina cadastrada ainda. Use o formulário acima para começar.</td></tr>"#.to_string();
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Boletim UNIP - Rust</title>
    <style>
        :root {{
            --bg: #0f172a;
            --card: #1e293b;
            --border: #334155;
            --text: #f8fafc;
            --muted: #94a3b8;
            --accent: #3b82f6;
            --accent-hover: #2563eb;
        }}
        * {{ box-sizing: border-box; font-family: system-ui, -apple-system, sans-serif; }}
        body {{ background: var(--bg); color: var(--text); margin: 0; padding: 2rem; }}
        .container {{ max-width: 1100px; margin: 0 auto; }}
        header {{
            display: flex; justify-content: space-between; align-items: center;
            background: var(--card); padding: 1.5rem; border-radius: 12px;
            border: 1px solid var(--border); margin-bottom: 1.5rem; flex-wrap: wrap; gap: 1rem;
        }}
        h1 {{ margin: 0; font-size: 1.5rem; }}
        .header-actions {{ display: flex; align-items: center; gap: 1rem; }}
        .btn-csv {{
            display: inline-block; background: #10b981; color: #052e16;
            text-decoration: none; padding: 0.75rem 1.1rem; border-radius: 8px;
            font-weight: 700; font-size: 0.9rem; transition: 0.2s;
        }}
        .btn-csv:hover {{ background: #34d399; }}
        .cr-box {{
            background: #0f172a; padding: 0.75rem 1.25rem; border-radius: 8px;
            border: 1px solid var(--accent); text-align: right;
        }}
        .cr-box span {{ display: block; font-size: 0.75rem; color: var(--muted); }}
        .cr-box strong {{ font-size: 1.5rem; color: var(--accent); }}
        .grid-forms {{
            display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1rem; margin-bottom: 1.5rem;
        }}
        .card {{
            background: var(--card); padding: 1.25rem; border-radius: 12px;
            border: 1px solid var(--border);
        }}
        .card h3 {{ margin-top: 0; font-size: 1.05rem; color: #e2e8f0; }}
        .form-row {{ display: flex; flex-direction: column; gap: 0.6rem; }}
        input, select {{
            background: #0f172a; border: 1px solid var(--border); color: var(--text);
            padding: 0.6rem; border-radius: 6px; font-size: 0.9rem; width: 100%;
        }}
        button {{
            background: var(--accent); color: white; border: none; padding: 0.65rem;
            border-radius: 6px; font-weight: 600; cursor: pointer; transition: 0.2s;
        }}
        button:hover {{ background: var(--accent-hover); }}
        .btn-del {{ background: #ef4444; padding: 0.3rem 0.6rem; font-size: 0.8rem; }}
        .btn-del:hover {{ background: #dc2626; }}
        .btn-reset {{
            background: #334155; color: #e2e8f0; padding: 0.15rem 0.45rem;
            font-size: 0.72rem; border-radius: 4px;
        }}
        .btn-reset:hover {{ background: #475569; }}
        table {{
            width: 100%; border-collapse: collapse; background: var(--card);
            border-radius: 12px; overflow: hidden; border: 1px solid var(--border);
        }}
        th, td {{ padding: 1rem; text-align: left; border-bottom: 1px solid var(--border); }}
        th {{ background: #0f172a; color: var(--muted); font-size: 0.85rem; text-transform: uppercase; }}
        .nota-chip {{
            display: inline-block; background: #0f172a; padding: 0.25rem 0.5rem;
            border-radius: 6px; font-size: 0.85rem; margin: 2px; border: 1px solid var(--border);
        }}
        .nota-chip.vazio {{ color: var(--muted); }}
        .badge {{
            padding: 0.25rem 0.6rem; border-radius: 999px; font-size: 0.75rem; font-weight: 700;
            display: inline-block;
        }}
        .badge-green {{ background: rgba(34, 197, 94, 0.2); color: #4ade80; }}
        .badge-yellow {{ background: rgba(234, 179, 8, 0.2); color: #facc15; }}
        .badge-red {{ background: rgba(239, 68, 68, 0.2); color: #f87171; }}
        .badge-blue {{ background: rgba(59, 130, 246, 0.2); color: #60a5fa; }}
        .meta {{ color: var(--muted); font-size: 0.78rem; }}
        @media print {{
            .grid-forms, .btn-del, .btn-csv, .btn-reset {{ display: none; }}
            body {{ background: white; color: black; }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <div>
                <h1>Boletim UNIP — {}</h1>
                <form action="/aluno" method="post" style="display:flex; gap:0.5rem; margin-top:0.5rem;">
                    <input type="text" name="aluno" placeholder="Alterar nome do aluno..." style="max-width:220px; padding:0.35rem 0.6rem;">
                    <button type="submit" style="padding:0.35rem 0.8rem; font-size:0.85rem;">Salvar</button>
                </form>
            </div>
            <div class="header-actions">
                <a href="/exportar-csv" class="btn-csv">⬇ Exportar CSV</a>
                <div class="cr-box">
                    <span>MÉDIA GERAL (CR)</span>
                    <strong>{:.2}</strong>
                </div>
            </div>
        </header>

        <div class="grid-forms">
            <div class="card">
                <h3>1. Nova Disciplina</h3>
                <form action="/disciplina" method="post" class="form-row">
                    <input type="text" name="nome" placeholder="Ex: Engenharia de Software" required>
                    <input type="number" name="qtd_avaliacoes" placeholder="Qtd. de Avaliações (ex: 2 ou 4)" min="1" max="10" value="2" required>
                    <input type="number" name="aulas_totais" placeholder="Carga horária / total aulas (ex: 60)" min="1" value="60" required>
                    <button type="submit">Cadastrar Disciplina</button>
                </form>
            </div>

            <div class="card">
                <h3>2. Lançar / Editar Nota</h3>
                <form action="/nota" method="post" class="form-row">
                    <select name="disciplina" required>{}</select>
                    <input type="number" name="avaliacao" placeholder="Nº da Avaliação (1, 2...)" min="1" required>
                    <input type="text" name="nota" placeholder="Nota (ex: 8.5 ou 8,5)" required>
                    <button type="submit">Lançar Nota</button>
                </form>
            </div>

            <div class="card">
                <h3>3. Gerenciar Faltas</h3>
                <form action="/falta" method="post" class="form-row">
                    <select name="disciplina" required>{}</select>
                    <select name="modo">
                        <option value="adicionar">Somar novas faltas (+)</option>
                        <option value="definir">Corrigir total exato (=)</option>
                    </select>
                    <input type="number" name="faltas" placeholder="Quantidade (ex: 2 ou 0 para zerar)" min="0" required>
                    <button type="submit">Atualizar Faltas</button>
                </form>
            </div>
        </div>

        <table>
            <thead>
                <tr>
                    <th>Disciplina</th>
                    <th>Avaliações</th>
                    <th>Média</th>
                    <th>Frequência</th>
                    <th>Situação / Meta</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        dados.aluno, cr, opcoes_select, opcoes_select, linhas_tabela
    );

    Html(html)
}
