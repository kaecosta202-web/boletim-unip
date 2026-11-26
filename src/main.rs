use axum::{
    extract::{Form, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    sync::{Arc, RwLock},
};

const PASTA_DADOS: &str = "dados";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Disciplina {
    nome: String,
    np1: Option<f64>,
    np2: Option<f64>,
    pim: Option<f64>,
    exame: Option<f64>,
    aulas_totais: u32,
    faltas: u32,
}

impl Disciplina {
    fn nova(nome: String, aulas_totais: u32) -> Self {
        Self {
            nome,
            np1: None,
            np2: None,
            pim: None,
            exame: None,
            aulas_totais: aulas_totais.max(1),
            faltas: 0,
        }
    }

    fn media_semestral(&self) -> f64 {
        let mut soma_ponderada = 0.0;
        let mut total_peso = 0.0;

        if let Some(n1) = self.np1 {
            soma_ponderada += 4.0 * n1;
            total_peso += 4.0;
        }
        if let Some(n2) = self.np2 {
            soma_ponderada += 4.0 * n2;
            total_peso += 4.0;
        }
        if let Some(p) = self.pim {
            soma_ponderada += 2.0 * p;
            total_peso += 2.0;
        }

        if total_peso == 0.0 {
            0.0
        } else {
            soma_ponderada / total_peso
        }
    }

    fn media_final(&self) -> f64 {
        let ms = self.media_semestral();
        match self.exame {
            Some(e) => (ms + e) / 2.0,
            None => ms,
        }
    }

    fn frequencia_percentual(&self) -> f64 {
        let presencas = self.aulas_totais.saturating_sub(self.faltas);
        (presencas as f64 / self.aulas_totais as f64) * 100.0
    }

    fn percentual_faltas(&self) -> f64 {
        (self.faltas as f64 / self.aulas_totais as f64) * 100.0
    }

    fn limite_faltas(&self) -> u32 {
        (self.aulas_totais as f64 * 0.25).floor() as u32
    }

    fn status_faltas(&self) -> (String, &'static str) {
        let limite = self.limite_faltas();
        let faltas_restantes = limite.saturating_sub(self.faltas);
        let perc = self.percentual_faltas();

        if self.faltas >= limite || perc >= 25.0 {
            ("Reprovado por falta".to_string(), "badge-red")
        } else if perc >= 15.0 {
            (format!("Restam {} faltas", faltas_restantes), "badge-yellow")
        } else {
            (format!("Restam {} faltas", faltas_restantes), "badge-green")
        }
    }

    fn analise_desempenho(&self, media_aprovacao: f64) -> String {
        let limite = self.limite_faltas();
        if self.faltas >= limite || self.frequencia_percentual() < 75.0 {
            return "Reprovado por falta".to_string();
        }

        if self.np2.is_none() {
            let n1 = self.np1.unwrap_or(0.0);
            let p = self.pim.unwrap_or(0.0);
            // 4*n1 + 4*np2 + 2*p >= 10*media_aprovacao
            let np2_nec = (10.0 * media_aprovacao - 4.0 * n1 - 2.0 * p) / 4.0;
            if np2_nec <= 0.0 {
                return "Meta já atingida!".to_string();
            } else if np2_nec > 10.0 {
                return format!("Precisa {:.1} na NP2 (Impossível >10)", np2_nec);
            } else {
                return format!("Falta {:.1} na NP2 para média direta", np2_nec);
            }
        }

        let ms = self.media_semestral();
        if ms >= media_aprovacao {
            return "Aprovado direto!".to_string();
        }

        if let Some(ex) = self.exame {
            let mf = (ms + ex) / 2.0;
            if mf >= 5.0 {
                return format!("Aprovado no Exame (Final: {:.1})", mf);
            } else {
                return format!("Reprovado no Exame (Final: {:.1})", mf);
            }
        }

        if ms < 3.0 {
            return format!("Reprovado por média (MS: {:.1} < 3.0)", ms);
        }

        let ex_nec = (10.0 - ms).max(0.0).min(10.0);
        format!("Precisa {:.1} no Exame Final", ex_nec)
    }

    fn status(&self, media_aprovacao: f64) -> (&'static str, &'static str) {
        let limite = self.limite_faltas();
        if self.faltas >= limite || self.frequencia_percentual() < 75.0 {
            return ("Reprovado por falta", "badge-red");
        }

        if self.np1.is_none() && self.np2.is_none() && self.pim.is_none() {
            return ("Cursando", "badge-blue");
        }

        let ms = self.media_semestral();

        if ms >= media_aprovacao {
            return ("Aprovado", "badge-green");
        }

        if ms < 3.0 && self.np2.is_some() {
            return ("Reprovado", "badge-red");
        }

        match self.exame {
            Some(ex) => {
                if (ms + ex) / 2.0 >= 5.0 {
                    ("Aprovado (Exame)", "badge-green")
                } else {
                    ("Reprovado", "badge-red")
                }
            }
            None => {
                if self.np2.is_some() {
                    ("Exame", "badge-yellow")
                } else {
                    ("Cursando", "badge-blue")
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EstadoBoletim {
    ra: String,
    aluno: String,
    media_aprovacao: f64, // 6.0 (SEPI) ou 7.0 (Presencial)
    disciplinas: HashMap<String, Disciplina>,
}

impl EstadoBoletim {
    fn carregar(ra: &str) -> Self {
        let caminho = format!("{}/ra_{}.json", PASTA_DADOS, ra);
        if let Ok(conteudo) = fs::read_to_string(&caminho) {
            if let Ok(dados) = serde_json::from_str(&conteudo) {
                return dados;
            }
        }
        Self {
            ra: ra.to_string(),
            aluno: format!("Aluno {}", ra),
            media_aprovacao: 7.0,
            disciplinas: HashMap::new(),
        }
    }

    fn salvar(&self) {
        let _ = fs::create_dir_all(PASTA_DADOS);
        let caminho = format!("{}/ra_{}.json", PASTA_DADOS, self.ra);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(caminho, json);
        }
    }
}

struct GerenciadorUsuarios {
    usuarios: RwLock<HashMap<String, Arc<RwLock<EstadoBoletim>>>>,
}

impl GerenciadorUsuarios {
    fn obter_usuario(&self, ra: &str) -> Arc<RwLock<EstadoBoletim>> {
        let ra_sanitizado = ra
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
            
        if ra_sanitizado.is_empty() {
            return Arc::new(RwLock::new(EstadoBoletim::carregar("convidado")));
        }

        {
            let lido = self.usuarios.read().unwrap();
            if let Some(user) = lido.get(&ra_sanitizado) {
                return Arc::clone(user);
            }
        }

        let novo_user = Arc::new(RwLock::new(EstadoBoletim::carregar(&ra_sanitizado)));
        let mut escrito = self.usuarios.write().unwrap();
        escrito.insert(ra_sanitizado, Arc::clone(&novo_user));
        novo_user
    }

    fn substituir_estado(&self, ra: &str, novo_estado: EstadoBoletim) {
        let ra_sanitizado = ra.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let user_state = self.obter_usuario(&ra_sanitizado);
        let mut u = user_state.write().unwrap();
        *u = novo_estado;
        u.ra = ra_sanitizado;
        u.salvar();
    }
}

type AppState = Arc<GerenciadorUsuarios>;

#[derive(Deserialize)]
struct LoginForm {
    ra: String,
}

#[derive(Deserialize)]
struct AlunoForm {
    aluno: String,
}

#[derive(Deserialize)]
struct MediaAprovacaoForm {
    media_aprovacao: f64,
}

#[derive(Deserialize)]
struct NovaDisciplinaForm {
    nome: String,
    aulas_totais: u32,
}

#[derive(Deserialize)]
struct LancarNotaForm {
    disciplina: String,
    tipo: String,
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

#[derive(Deserialize)]
struct RestaurarJsonForm {
    json_content: String,
}

#[tokio::main]
async fn main() {
    let _ = fs::create_dir_all(PASTA_DADOS);
    let estado = Arc::new(GerenciadorUsuarios {
        usuarios: RwLock::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/", get(pagina_inicial))
        .route("/entrar", post(login))
        .route("/sair", get(logout))
        .route("/exportar-csv", get(exportar_csv))
        .route("/backup.json", get(exportar_backup_json))
        .route("/restaurar-json", post(restaurar_json))
        .route("/aluno", post(atualizar_aluno))
        .route("/media-aprovacao", post(atualizar_media_aprovacao))
        .route("/disciplina", post(adicionar_disciplina))
        .route("/nota", post(lancar_nota))
        .route("/falta", post(gerenciar_falta))
        .route("/zerar-faltas", post(zerar_faltas))
        .route("/remover", post(remover_disciplina))
        .with_state(estado);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("🚀 Boletim UNIP rodando em: http://localhost:8080");

    axum::serve(listener, app).await.unwrap();
}

fn obter_ra_do_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find(|s| s.trim().starts_with("ra="))?
        .split('=')
        .nth(1)?
        .to_string()
        .into()
}

async fn login(Form(form): Form<LoginForm>) -> impl IntoResponse {
    let ra = form.ra.chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    if ra.is_empty() {
        return Redirect::to("/").into_response();
    }
    
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/")
        .header(header::SET_COOKIE, format!("ra={}; Path=/; HttpOnly; SameSite=Lax", ra))
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

async fn logout() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/")
        .header(header::SET_COOKIE, "ra=; Path=/; Max-Age=0")
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response()
}

async fn pagina_inicial(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let ra = match obter_ra_do_cookie(&headers) {
        Some(ra) => ra,
        None => return Html(format!(
            r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Entrar - Boletim UNIP</title>
    <style>
        body {{ background: #0f172a; color: #f8fafc; font-family: system-ui, sans-serif; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
        .login-card {{ background: #1e293b; padding: 2.5rem; border-radius: 16px; border: 1px solid #334155; width: 100%; max-width: 400px; text-align: center; }}
        h1 {{ margin-bottom: 1.5rem; font-size: 1.8rem; }}
        input {{ background: #0f172a; border: 1px solid #334155; color: white; padding: 0.8rem; border-radius: 8px; width: 100%; margin-bottom: 1rem; box-sizing: border-box; }}
        button {{ background: #3b82f6; color: white; border: none; padding: 0.8rem; border-radius: 8px; width: 100%; font-weight: 700; cursor: pointer; }}
        button:hover {{ background: #2563eb; }}
        p {{ color: #94a3b8; font-size: 0.9rem; margin-top: 1.5rem; }}
    </style>
</head>
<body>
    <div class="login-card">
        <h1>Boletim UNIP</h1>
        <form action="/entrar" method="post">
            <input type="text" name="ra" placeholder="Seu RA ou Apelido" required autofocus>
            <button type="submit">Acessar Boletim</button>
        </form>
        <p>Dados salvos individualmente por RA/Apelido.</p>
    </div>
</body>
</html>"#
        )),
    };

    let user_state = state.obter_usuario(&ra);
    let dados = user_state.read().unwrap();
    let mut lista: Vec<&Disciplina> = dados.disciplinas.values().collect();
    lista.sort_by(|a, b| a.nome.cmp(&b.nome));

    let cr = if lista.is_empty() {
        0.0
    } else {
        lista.iter().map(|d| d.media_semestral()).sum::<f64>() / (lista.len() as f64)
    };

    let mut opcoes_select = String::new();
    let mut linhas_tabela = String::new();

    for d in &lista {
        opcoes_select.push_str(&format!(
            "<option value=\"{}\">{} (NP1: {:?}, NP2: {:?}, PIM: {:?})</option>",
            d.nome, d.nome, d.np1, d.np2, d.pim
        ));

        let n1_str = match d.np1 { Some(v) => format!("{:.1}", v), None => "--".to_string() };
        let n2_str = match d.np2 { Some(v) => format!("{:.1}", v), None => "--".to_string() };
        let pim_str = match d.pim { Some(v) => format!("{:.1}", v), None => "--".to_string() };
        let ex_str = match d.exame { Some(v) => format!("{:.1}", v), None => "--".to_string() };

        let notas_html = format!(
            "<span class='nota-chip'>NP1: <b>{}</b></span> <span class='nota-chip'>NP2: <b>{}</b></span> <span class='nota-chip'>PIM: <b>{}</b></span> <span class='nota-chip'>Exame: <b>{}</b></span>",
            n1_str, n2_str, pim_str, ex_str
        );

        let n1_val = d.np1.map(|v| v.to_string()).unwrap_or_default();
        let n2_val = d.np2.map(|v| v.to_string()).unwrap_or_default();
        let pim_val = d.pim.map(|v| v.to_string()).unwrap_or_default();
        let sim_id = d.nome.chars().filter(|c| c.is_alphanumeric()).collect::<String>();

        let simulador_html = format!(
            r#"<div style="font-size:0.75rem; color:var(--muted); margin-bottom:2px">Simular (NP1, NP2, PIM):</div>
            <div style="display:flex; gap:3px; align-items:center;">
                <input type="number" step="0.1" min="0" max="10" placeholder="NP1" value="{}" id="sim_n1_{}" style="width:45px; padding:2px; font-size:0.75rem" oninput="calcularSimulacao('{}')">
                <input type="number" step="0.1" min="0" max="10" placeholder="NP2" value="{}" id="sim_n2_{}" style="width:45px; padding:2px; font-size:0.75rem" oninput="calcularSimulacao('{}')">
                <input type="number" step="0.1" min="0" max="10" placeholder="PIM" value="{}" id="sim_pim_{}" style="width:45px; padding:2px; font-size:0.75rem" oninput="calcularSimulacao('{}')">
            </div>
            <div style="font-size:0.75rem; margin-top:3px;" id="sim_res_{}">Simulada: <b>{:.2}</b></div>"#,
            n1_val, sim_id, sim_id, n2_val, sim_id, sim_id, pim_val, sim_id, sim_id, sim_id, d.media_semestral()
        );

        let media_txt = if d.exame.is_some() {
            format!("{:.2} <small class='meta'>(Final: {:.2})</small>", d.media_semestral(), d.media_final())
        } else {
            format!("{:.2}", d.media_semestral())
        };

        let (falta_txt, falta_badge) = d.status_faltas();
        let freq_html = format!(
            "{:.1}% <small>({}/{})</small><br><span class='badge {}' style='margin-top:4px; display:inline-block;'>{}</span>",
            d.frequencia_percentual(),
            d.aulas_totais.saturating_sub(d.faltas),
            d.aulas_totais,
            falta_badge,
            falta_txt
        );

        let (status_txt, badge_class) = d.status(dados.media_aprovacao);
        let analise = d.analise_desempenho(dados.media_aprovacao);
        let situacao_html = format!(
            "<span class='badge {}'>{}</span><br><small class='meta' style='margin-top:4px; display:block;'>{}</small>",
            badge_class, status_txt, analise
        );

        let btn_zerar = if d.faltas > 0 {
            format!(
                r#"<form action="/zerar-faltas" method="post" style="display:inline; margin-right:4px;">
                    <input type="hidden" name="disciplina" value="{}">
                    <button type="submit" class="btn-reset" title="Zerar faltas">Zerar</button>
                </form>"#, d.nome
            )
        } else { String::new() };

        let acoes_html = format!(
            r#"{}
            <form action="/remover" method="post" style="display:inline; margin:0;">
                <input type="hidden" name="disciplina" value="{}">
                <button type="submit" class="btn-del" title="Remover disciplina">✕</button>
            </form>"#,
            btn_zerar, d.nome
        );

        linhas_tabela.push_str(&format!(
            r#"<tr>
                <td><strong>{}</strong></td>
                <td>{}</td>
                <td>{}</td>
                <td><strong>{}</strong></td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
            </tr>"#,
            d.nome, notas_html, simulador_html, media_txt, freq_html, situacao_html, acoes_html
        ));
    }

    if lista.is_empty() {
        linhas_tabela = r#"<tr><td colspan="7" style="text-align:center; color:#94a3b8; padding:2rem;">Nenhuma disciplina cadastrada.</td></tr>"#.to_string();
    }

    let sel_presencial = if dados.media_aprovacao == 7.0 { "selected" } else { "" };
    let sel_sepi = if dados.media_aprovacao == 6.0 { "selected" } else { "" };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Boletim UNIP - {}</title>
    <style>
        :root {{ --bg: #0f172a; --card: #1e293b; --border: #334155; --text: #f8fafc; --muted: #94a3b8; --accent: #3b82f6; }}
        * {{ box-sizing: border-box; font-family: system-ui, sans-serif; }}
        body {{ background: var(--bg); color: var(--text); margin: 0; padding: 2rem; }}
        .container {{ max-width: 1300px; margin: 0 auto; }}
        header {{ display: flex; justify-content: space-between; align-items: center; background: var(--card); padding: 1.5rem; border-radius: 12px; border: 1px solid var(--border); margin-bottom: 1.5rem; flex-wrap: wrap; gap: 1rem; }}
        .user-info {{ display: flex; flex-direction: column; gap: 0.5rem; }}
        .btn-sair {{ color: #ef4444; text-decoration: none; font-size: 0.85rem; font-weight: 600; }}
        .btn-csv {{ background: #10b981; color: #052e16; text-decoration: none; padding: 0.6rem 1rem; border-radius: 8px; font-weight: 700; font-size: 0.85rem; display: inline-flex; align-items: center; gap: 0.4rem; }}
        .btn-backup {{ background: #6366f1; color: #ffffff; text-decoration: none; padding: 0.6rem 1rem; border-radius: 8px; font-weight: 700; font-size: 0.85rem; display: inline-flex; align-items: center; gap: 0.4rem; }}
        .cr-box {{ background: #0f172a; padding: 0.75rem 1.25rem; border-radius: 8px; border: 1px solid var(--accent); text-align: right; }}
        .cr-box span {{ display: block; font-size: 0.75rem; color: var(--muted); }}
        .cr-box strong {{ font-size: 1.5rem; color: var(--accent); }}
        .grid-forms {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 1rem; margin-bottom: 1.5rem; }}
        .card {{ background: var(--card); padding: 1.25rem; border-radius: 12px; border: 1px solid var(--border); }}
        input, select, button {{ background: #0f172a; border: 1px solid var(--border); color: var(--text); padding: 0.6rem; border-radius: 6px; width: 100%; }}
        button {{ background: var(--accent); font-weight: 600; cursor: pointer; border: none; }}
        button:hover {{ opacity: 0.9; }}
        .btn-del {{ background: #ef4444; width: auto; padding: 0.3rem 0.6rem; font-size: 0.8rem; }}
        .btn-reset {{ background: #334155; width: auto; padding: 0.3rem 0.5rem; font-size: 0.75rem; }}
        table {{ width: 100%; border-collapse: collapse; background: var(--card); border-radius: 12px; overflow: hidden; border: 1px solid var(--border); }}
        th, td {{ padding: 1rem; text-align: left; border-bottom: 1px solid var(--border); vertical-align: middle; }}
        th {{ background: #0f172a; color: var(--muted); font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.05em; }}
        .nota-chip {{ display: inline-block; background: #0f172a; padding: 0.2rem 0.5rem; border-radius: 6px; font-size: 0.8rem; margin: 2px 0; border: 1px solid var(--border); }}
        .badge {{ padding: 0.25rem 0.6rem; border-radius: 999px; font-size: 0.75rem; font-weight: 700; white-space: nowrap; }}
        .badge-green {{ background: rgba(34, 197, 94, 0.2); color: #4ade80; }}
        .badge-yellow {{ background: rgba(234, 179, 8, 0.2); color: #facc15; }}
        .badge-red {{ background: rgba(239, 68, 68, 0.2); color: #f87171; }}
        .badge-blue {{ background: rgba(59, 130, 246, 0.2); color: #60a5fa; }}
        .meta {{ color: var(--muted); font-size: 0.75rem; }}
        .backup-section {{ background: var(--card); padding: 1rem; border-radius: 12px; border: 1px solid var(--border); margin-bottom: 1.5rem; display: flex; gap: 1rem; align-items: center; flex-wrap: wrap; justify-content: space-between; }}
    </style>
    <script>
        const mediaAprovacaoAtual = {};

        function calcularSimulacao(id) {{
            const n1 = parseFloat(document.getElementById('sim_n1_' + id).value) || 0;
            const n2 = parseFloat(document.getElementById('sim_n2_' + id).value) || 0;
            const p = parseFloat(document.getElementById('sim_pim_' + id).value) || 0;
            
            const media = (4.0 * n1 + 4.0 * n2 + 2.0 * p) / 10.0;
            
            let texto = "";
            let cls = "";
            if (media >= mediaAprovacaoAtual) {{
                texto = "Aprovado Direto";
                cls = "color:#4ade80";
            }} else if (media >= 3.0) {{
                const exameNec = Math.max(0, 10.0 - media).toFixed(1);
                texto = `Exame (Precisa ${{exameNec}})`;
                cls = "color:#facc15";
            }} else {{
                texto = "Reprovado por Média";
                cls = "color:#f87171";
            }}
            
            document.getElementById('sim_res_' + id).innerHTML = `Simulada: <b>${{media.toFixed(2)}}</b> - <span style="${{cls}}">${{texto}}</span>`;
        }}

        function carregarArquivoJson(event) {{
            const file = event.target.files[0];
            if (!file) return;
            const reader = new FileReader();
            reader.onload = function(e) {{
                document.getElementById('jsonContentInput').value = e.target.result;
            }};
            reader.readAsText(file);
        }}
    </script>
</head>
<body>
    <div class="container">
        <header>
            <div class="user-info">
                <h1 style="margin:0">{}</h1>
                <div>
                    <span style="color:var(--muted); font-size:0.9rem">RA: {}</span>
                    <a href="/sair" class="btn-sair" style="margin-left:1rem">Trocar Aluno / Sair</a>
                </div>
                <form action="/aluno" method="post" style="display:flex; gap:0.5rem; margin-top:0.5rem;">
                    <input type="text" name="aluno" placeholder="Nome do Aluno" style="max-width:200px; padding:0.3rem">
                    <button type="submit" style="width:auto; padding:0.3rem 0.8rem">Salvar Nome</button>
                </form>
                <form action="/media-aprovacao" method="post" style="display:flex; gap:0.5rem; align-items:center; margin-top:0.2rem;">
                    <span style="font-size:0.8rem; color:var(--muted)">Tipo/Média:</span>
                    <select name="media_aprovacao" onchange="this.form.submit()" style="padding:0.25rem; width:auto; font-size:0.85rem">
                        <option value="7.0" {}>Presencial (Média 7.0)</option>
                        <option value="6.0" {}>SEPI / EaD (Média 6.0)</option>
                    </select>
                </form>
            </div>
            <div style="display:flex; align-items:center; gap:1.5rem">
                <div class="cr-box"><span>MÉDIA GERAL (CR)</span><strong>{:.2}</strong></div>
            </div>
        </header>

        <div class="backup-section">
            <div style="display:flex; gap:1rem; align-items:center; flex-wrap:wrap;">
                <a href="/backup.json" class="btn-backup">⬇ Baixar Backup JSON</a>
                <a href="/exportar-csv" class="btn-csv">⬇ Exportar CSV</a>
            </div>
            <form action="/restaurar-json" method="post" style="display:flex; gap:0.5rem; align-items:center; flex:1; min-width:300px; justify-content:flex-end;">
                <input type="file" id="jsonFile" accept=".json" onchange="carregarArquivoJson(event)" style="font-size:0.8rem; max-width:180px; padding:0.2rem">
                <input type="text" name="json_content" id="jsonContentInput" placeholder="Ou cole o JSON do backup aqui..." style="font-size:0.8rem; padding:0.4rem; max-width:300px;" required>
                <button type="submit" style="width:auto; padding:0.4rem 0.8rem; font-size:0.8rem; background:#10b981">Restaurar JSON</button>
            </form>
        </div>

        <div class="grid-forms">
            <div class="card">
                <h3>Nova Disciplina</h3>
                <form action="/disciplina" method="post" style="display:flex; flex-direction:column; gap:0.5rem">
                    <input type="text" name="nome" placeholder="Nome da Disciplina" required>
                    <input type="number" name="aulas_totais" placeholder="Total de Aulas (ex: 60)" min="1" value="60" required>
                    <button type="submit">Cadastrar Disciplina</button>
                </form>
            </div>
            <div class="card">
                <h3>Lançar Nota (NP1, NP2, PIM, Exame)</h3>
                <form action="/nota" method="post" style="display:flex; flex-direction:column; gap:0.5rem">
                    <select name="disciplina" required>{}</select>
                    <select name="tipo" required>
                        <option value="np1">NP1 (Peso 4)</option>
                        <option value="np2">NP2 (Peso 4)</option>
                        <option value="pim">PIM (Peso 2)</option>
                        <option value="exame">Exame Final</option>
                    </select>
                    <input type="text" name="nota" placeholder="Nota (ex: 8.5)" required>
                    <button type="submit">Lançar Nota</button>
                </form>
            </div>
            <div class="card">
                <h3>Gerenciar Faltas</h3>
                <form action="/falta" method="post" style="display:flex; flex-direction:column; gap:0.5rem">
                    <select name="disciplina" required>{}</select>
                    <select name="modo"><option value="adicionar">Somar Faltas (+)</option><option value="definir">Definir Total (=)</option></select>
                    <input type="number" name="faltas" placeholder="Qtd de Faltas" min="0" required>
                    <button type="submit">Atualizar Faltas</button>
                </form>
            </div>
        </div>
        <table>
            <thead>
                <tr>
                    <th>Disciplina</th>
                    <th>Notas Oficiais</th>
                    <th>Simulador Rápido (JS)</th>
                    <th>Média</th>
                    <th>Frequência & Faltas</th>
                    <th>Situação & Meta</th>
                    <th>Ações</th>
                </tr>
            </thead>
            <tbody>{}</tbody>
        </table>
    </div>
</body>
</html>"#,
        dados.aluno, dados.media_aprovacao, dados.aluno, ra, sel_presencial, sel_sepi, cr,
        opcoes_select, opcoes_select, linhas_tabela
    );

    Html(html)
}

async fn exportar_csv(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let ra = match obter_ra_do_cookie(&headers) {
        Some(ra) => ra,
        None => return (StatusCode::UNAUTHORIZED, "Não autorizado").into_response(),
    };
    
    let user_state = state.obter_usuario(&ra);
    let dados = user_state.read().unwrap();
    let mut lista: Vec<&Disciplina> = dados.disciplinas.values().collect();
    lista.sort_by(|a, b| a.nome.cmp(&b.nome));

    let mut csv = String::from("\u{FEFF}RA;Aluno;Disciplina;NP1;NP2;PIM;Exame;MediaSemestral;MediaFinal;Faltas;Frequencia;Situacao\n");
    for d in &lista {
        let n1 = d.np1.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "--".to_string());
        let n2 = d.np2.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "--".to_string());
        let pim = d.pim.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "--".to_string());
        let ex = d.exame.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "--".to_string());
        let (status, _) = d.status(dados.media_aprovacao);
        csv.push_str(&format!(
            "{};{};{};{};{};{};{};{:.2};{:.2};{};{:.1}%;{}\n",
            ra, dados.aluno, d.nome, n1, n2, pim, ex, d.media_semestral(), d.media_final(), d.faltas, d.frequencia_percentual(), status
        ).replace('.', ","));
    }
    
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"boletim.csv\"")], csv).into_response()
}

async fn exportar_backup_json(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let ra = match obter_ra_do_cookie(&headers) {
        Some(ra) => ra,
        None => return (StatusCode::UNAUTHORIZED, "Não autorizado").into_response(),
    };
    
    let user_state = state.obter_usuario(&ra);
    let dados = user_state.read().unwrap();
    
    if let Ok(json) = serde_json::to_string_pretty(&*dados) {
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json; charset=utf-8"),
                (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"backup_boletim_{}.json\"", ra)),
            ],
            json,
        ).into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "Erro ao gerar backup").into_response()
    }
}

async fn restaurar_json(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<RestaurarJsonForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados_importados) = serde_json::from_str::<EstadoBoletim>(&form.json_content) {
            dados_importados.ra = ra.clone();
            dados_importados.salvar();
            state.substituir_estado(&ra, dados_importados);
        }
    }
    Redirect::to("/")
}

async fn atualizar_aluno(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<AlunoForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            let nome = form.aluno.trim();
            if !nome.is_empty() {
                dados.aluno = nome.to_string();
                dados.salvar();
            }
        }
    }
    Redirect::to("/")
}

async fn atualizar_media_aprovacao(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<MediaAprovacaoForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            if form.media_aprovacao == 6.0 || form.media_aprovacao == 7.0 {
                dados.media_aprovacao = form.media_aprovacao;
                dados.salvar();
            }
        }
    }
    Redirect::to("/")
}

async fn adicionar_disciplina(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<NovaDisciplinaForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            let nome = form.nome.trim();
            if !nome.is_empty() {
                dados.disciplinas.insert(
                    nome.to_lowercase(),
                    Disciplina::nova(nome.to_string(), form.aulas_totais),
                );
                dados.salvar();
            }
        }
    }
    Redirect::to("/")
}

async fn lancar_nota(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<LancarNotaForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            if let Some(disc) = dados.disciplinas.get_mut(&form.disciplina.to_lowercase()) {
                if let Ok(valor) = form.nota.replace(',', ".").trim().parse::<f64>() {
                    if (0.0..=10.0).contains(&valor) {
                        match form.tipo.as_str() {
                            "np1" => disc.np1 = Some(valor),
                            "np2" => disc.np2 = Some(valor),
                            "pim" => disc.pim = Some(valor),
                            "exame" => disc.exame = Some(valor),
                            _ => {}
                        }
                        dados.salvar();
                    }
                }
            }
        }
    }
    Redirect::to("/")
}

async fn gerenciar_falta(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<GerenciarFaltaForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            if let Some(disc) = dados.disciplinas.get_mut(&form.disciplina.to_lowercase()) {
                if form.modo == "definir" {
                    disc.faltas = form.faltas.min(disc.aulas_totais);
                } else {
                    disc.faltas = (disc.faltas + form.faltas).min(disc.aulas_totais);
                }
                dados.salvar();
            }
        }
    }
    Redirect::to("/")
}

async fn zerar_faltas(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<DisciplinaUnicaForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            if let Some(disc) = dados.disciplinas.get_mut(&form.disciplina.to_lowercase()) {
                disc.faltas = 0;
                dados.salvar();
            }
        }
    }
    Redirect::to("/")
}

async fn remover_disciplina(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<DisciplinaUnicaForm>) -> Redirect {
    if let Some(ra) = obter_ra_do_cookie(&headers) {
        if let Ok(mut dados) = state.obter_usuario(&ra).write() {
            dados.disciplinas.remove(&form.disciplina.to_lowercase());
            dados.salvar();
        }
    }
    Redirect::to("/")
}
