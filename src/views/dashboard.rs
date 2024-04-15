use iced::{
    theme,
    widget::{
        button,
        pane_grid::{self, Configuration},
        responsive, row, text, PaneGrid,
    },
    window, Command, Element, Font, Length,
};
use iced_futures::Subscription;
use ringbuf::Rb;
use tokio::sync::mpsc;

use crate::{
    api::Client,
    app::AppData,
    config::Config,
    message::Message,
    theme::h2c,
    ws::{book, trades, WsEvent, WsMessage},
};

use super::panes::{
    balances::balances_view,
    book::book_view,
    calculator::{CalculatorMessage, CalculatorPane},
    chart::ChartPane,
    market::market_view,
    orders::orders_view,
    style,
    trades::trades_view,
    watchlist::{watchlist_view, WatchlistFilter},
};

#[derive(PartialEq)]
pub enum PaneType {
    Prices,
    Book,
    Trades,
    Market,
    Balances,
    Orders,
    Calculator,
    Chart,
}

impl From<usize> for PaneType {
    fn from(_value: usize) -> Self {
        Self::Balances
    }
}

impl ToString for PaneType {
    fn to_string(&self) -> String {
        match self {
            PaneType::Prices => "Watchlist",
            PaneType::Book => "Book",
            PaneType::Trades => "Trades",
            PaneType::Market => "Market",
            PaneType::Balances => "Balances",
            PaneType::Orders => "Orders",
            PaneType::Calculator => "Calculator",
            PaneType::Chart => "Chart",
        }
        .to_string()
    }
}

pub struct Pane {
    pub id: PaneType,
    pub is_pinned: bool,
}

impl Pane {
    pub fn new(ty: PaneType) -> Self {
        Self {
            id: ty,
            is_pinned: false,
        }
    }
}

pub fn view_controls<'a>(
    pane: pane_grid::Pane,
    total_panes: usize,
    is_pinned: bool,
    is_maximized: bool,
) -> Element<'a, Message> {
    let mut row = row![].spacing(5);

    if total_panes > 1 {
        let toggle = {
            let (content, message) = if is_maximized {
                (
                    text('\u{F3DE}').font(Font::with_name("bootstrap-icons")),
                    DashboardMessage::Restore.into(),
                )
            } else {
                (
                    text('\u{F3DF}').font(Font::with_name("bootstrap-icons")),
                    DashboardMessage::Maximize(pane).into(),
                )
            };
            button(content.size(12).style(h2c("FFFFFF").unwrap()))
                .height(14)
                .width(14)
                .style(theme::Button::Secondary)
                .on_press(message)
        };

        row = row.push(toggle);
    }

    let mut close = button(
        text('\u{F62A}')
            .size(12)
            .font(Font::with_name("bootstrap-icons")),
    )
    .height(14)
    .width(14)
    .style(theme::Button::Destructive);

    if total_panes > 1 && !is_pinned {
        close = close.on_press(DashboardMessage::Close(pane).into());
    }

    row.push(close).into()
}

#[derive(Debug, Clone)]
pub(crate) enum DashboardMessage {
    Clicked(pane_grid::Pane),
    Dragged(pane_grid::DragEvent),
    Resized(pane_grid::ResizeEvent),
    Maximize(pane_grid::Pane),
    Restore,
    Close(pane_grid::Pane),
    WatchlistFilterInput(String),
    ApplyWatchlistFilter(WatchlistFilter),
    SellPressed,
    BuyPressed,
    MarketQuoteChanged(String),
    MarketAmtChanged(String),
    MarketPairChanged(String),
    MarketPairSet,
    PriceInc(f64),
    AssetSelected(String),
    QtySet(f64),
    Calculator(CalculatorMessage),
}

impl From<CalculatorMessage> for DashboardMessage {
    fn from(value: CalculatorMessage) -> Self {
        Self::Calculator(value)
    }
}

pub(crate) struct DashboardView {
    focus: Option<pane_grid::Pane>,
    panes: pane_grid::State<Pane>,
    chart: ChartPane,
    calculator: CalculatorPane,
    // ???
    filter: WatchlistFilter,
    filter_string: String,
    // market widget values
    textbox_price: String,
    textbox_amount: String,
    pub textbox_pair: String,
    // websockets
    ws_book: Option<mpsc::UnboundedSender<book::Message>>,
    ws_trade: Option<mpsc::UnboundedSender<trades::Message>>,
}

macro_rules! v {
    ($r: expr, $a: expr, $b: expr) => {
        b![Vertical, $r, $a, $b]
    };
}
macro_rules! h {
    ($r: expr, $a: expr, $b: expr) => {
        b![Horizontal, $r, $a, $b]
    };
}
macro_rules! b {
    ($d: ident, $r: expr, $a: expr, $b: expr) => {
        Configuration::Split {
            axis: pane_grid::Axis::$d,
            ratio: $r,
            a: Box::new($a),
            b: Box::new($b),
        }
    };
}
macro_rules! pane {
    ($p: ident) => {
        Configuration::Pane(Pane::new(PaneType::$p))
    };
}

impl DashboardView {
    pub(crate) fn new() -> Self {
        let panes = pane_grid::State::with_configuration(h![
            0.65,
            v![
                0.15,
                h![0.6, pane![Prices], pane![Balances]],
                v![
                    0.5,
                    pane![Chart],
                    v![0.6, h![0.33, pane![Market], pane![Trades]], pane![Book]]
                ]
            ],
            v![0.7, pane![Orders], pane![Calculator]]
        ]);

        Self {
            focus: None,
            panes,
            chart: ChartPane::new(),
            calculator: CalculatorPane::new(),
            filter: WatchlistFilter::Favorites,
            filter_string: "".to_string(),
            textbox_price: Default::default(),
            textbox_amount: Default::default(),
            textbox_pair: "BTCUSDT".into(),
            ws_book: None,
            ws_trade: None,
        }
    }

    pub(crate) fn update(
        &mut self,
        message: DashboardMessage,
        api: &Client,
        data: &AppData,
    ) -> Command<Message> {
        match message {
            DashboardMessage::Clicked(pane) => {
                self.focus = Some(pane);
                Command::none()
            }
            DashboardMessage::Resized(pane_grid::ResizeEvent { split, ratio }) => {
                self.panes.resize(split, ratio);
                Command::none()
            }
            DashboardMessage::Dragged(pane_grid::DragEvent::Dropped { pane, target }) => {
                self.panes.drop(pane, target);
                Command::none()
            }
            DashboardMessage::Dragged(_) => Command::none(),
            DashboardMessage::Maximize(pane) => {
                self.panes.maximize(pane);
                Command::none()
            }
            DashboardMessage::Restore => {
                self.panes.restore();
                Command::none()
            }
            DashboardMessage::Close(pane) => {
                if let Some((_, sibling)) = self.panes.close(pane) {
                    self.focus = Some(sibling);
                }
                Command::none()
            }
            DashboardMessage::ApplyWatchlistFilter(f) => {
                self.filter = f;
                Command::none()
            }
            DashboardMessage::WatchlistFilterInput(wfi) => {
                self.filter_string = wfi;
                Command::none()
            }
            DashboardMessage::BuyPressed => api.trade_spot(
                self.textbox_pair.clone(),
                self.textbox_price.parse().unwrap(),
                self.textbox_amount.parse().unwrap(),
                binance::rest_model::OrderSide::Buy,
            ),
            DashboardMessage::SellPressed => api.trade_spot(
                self.textbox_pair.clone(),
                self.textbox_price.parse().unwrap(),
                self.textbox_amount.parse().unwrap(),
                binance::rest_model::OrderSide::Sell,
            ),
            DashboardMessage::MarketPairChanged(np) => {
                self.textbox_pair = np;
                Command::none()
            }
            DashboardMessage::MarketQuoteChanged(nm) => {
                self.textbox_price = nm;
                Command::none()
            }
            DashboardMessage::MarketAmtChanged(nm) => {
                self.textbox_amount = nm;
                Command::none()
            }
            DashboardMessage::AssetSelected(a) => {
                if !a.ends_with("USDT") && !a.ends_with("BTC") && !a.ends_with("ETH") {
                    self.textbox_pair = format!("{a}USDT");
                } else {
                    self.textbox_pair = a;
                };

                self.update_pair();

                Command::none()
            }
            DashboardMessage::QtySet(f) => {
                let usdt_b = data
                    .balances
                    .iter()
                    .find(|b| b.asset == "USDT")
                    .unwrap()
                    .free;
                self.textbox_amount = (usdt_b * f).to_string();
                Command::none()
            }
            DashboardMessage::PriceInc(inc) => {
                let price = data
                    .prices
                    .as_ref()
                    .expect("prices exist for some reason")
                    .get(&self.textbox_pair)
                    .expect("price exists for some reason");
                self.textbox_price =
                    (((*price as f64 * (1.0 + (inc / 100.0))) * 100.0).round() / 100.0).to_string();
                Command::none()
            }
            DashboardMessage::Calculator(msg) => self.calculator.update(msg),
            DashboardMessage::MarketPairSet => {
                self.update_pair();
                Command::none()
            }
        }
    }

    pub(crate) fn tick(&mut self, data: &AppData) {
        self.calculator.tick(data)
    }

    pub(crate) fn prepend_chart_data(&mut self, slc: &[f64]) -> Command<Message> {
        self.chart.data.clear();
        self.chart.data.push_slice_overwrite(slc);
        Command::none()
    }

    pub(crate) fn ws(&mut self, message: WsMessage) -> Command<Message> {
        match message {
            WsMessage::Price(m) => match m {
                crate::ws::WsEvent::Connected(_) => (),
                crate::ws::WsEvent::Disconnected => (),
                crate::ws::WsEvent::Message(m) => {
                    if m.name == self.textbox_pair {
                        self.chart.update_data(m.price.into());
                    }
                }
            },
            WsMessage::Book(m) => match m {
                WsEvent::Connected(sender) => self.ws_book = Some(sender),
                WsEvent::Disconnected => self.ws_book = None,
                WsEvent::Message(_) => (),
            },
            WsMessage::Trade(m) => match m {
                WsEvent::Connected(sender) => self.ws_trade = Some(sender),
                WsEvent::Disconnected => self.ws_trade = None,
                WsEvent::Message(_) => (),
            },
            WsMessage::User(_) => (),
        }

        Command::none()
    }

    pub(crate) fn view<'a>(&'a self, data: &'a AppData, config: &'a Config) -> PaneGrid<Message> {
        let focus = self.focus;
        let total_panes = self.panes.len();

        PaneGrid::new(&self.panes, |id, pane, is_maximized| {
            let is_focused = focus == Some(id);

            let title = row![text(pane.id.to_string())].spacing(5);
            let title_bar = pane_grid::TitleBar::new(title)
                .controls(view_controls(id, total_panes, pane.is_pinned, is_maximized))
                .padding([8, 12]);

            pane_grid::Content::new(responsive(|_size| match pane.id {
                PaneType::Prices => watchlist_view(
                    &data.prices,
                    &config.watchlist_favorites,
                    self.filter,
                    &self.filter_string,
                    &data.loader,
                ),
                PaneType::Chart => self.chart.view().into(),
                PaneType::Book => book_view(&data.book),
                PaneType::Trades => trades_view(&data.trades),
                PaneType::Market => market_view(
                    &self.textbox_price,
                    &self.textbox_amount,
                    &self.textbox_pair,
                ),
                PaneType::Balances => balances_view(&data.balances),
                PaneType::Orders => orders_view(&data.orders, &data.prices),
                PaneType::Calculator => self.calculator.view(),
            }))
            .title_bar(title_bar)
            .style(if is_focused {
                style::pane_focused
            } else {
                style::pane_active
            })
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .spacing(10)
        .on_click(|p| DashboardMessage::Clicked(p).into())
        .on_drag(|d| DashboardMessage::Dragged(d).into())
        .on_resize(10, |r| DashboardMessage::Resized(r).into())
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            trades::connect(self.textbox_pair.to_lowercase()).map(Message::from),
            book::connect(self.textbox_pair.to_lowercase()).map(Message::from),
            window::frames().map(Message::LoaderTick),
        ])
    }

    fn update_pair(&mut self) {
        let lower_pair = self.textbox_pair.to_lowercase();

        if let Some(book_ws) = &mut self.ws_book {
            book_ws
                .send(book::Message::NewPair(lower_pair.clone()))
                .unwrap();
        };
        if let Some(ws_trade) = &mut self.ws_trade {
            ws_trade.send(trades::Message::NewPair(lower_pair)).unwrap();
        };
    }
}
