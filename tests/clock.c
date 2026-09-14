#include <adwaita.h>

static GDateTime *test_time;
static guint scheduled_seconds;
static GDateTime *test_now(void) { return g_date_time_ref(test_time); }
static guint test_timeout(guint interval, GSourceFunc callback, gpointer data) {
    return 1;
}
static guint test_seconds(gint priority, guint interval, GSourceFunc callback,
                          gpointer data, GDestroyNotify notify) {
    scheduled_seconds = interval;
    return 2;
}
#define g_date_time_new_now_local test_now
#define g_timeout_add test_timeout
#define g_timeout_add_seconds_full test_seconds
#include "../src/services/clock_service.c"

static void on_tick(ClockService *service, GDateTime *now, guint *ticks) {
    (*ticks)++;
    g_assert_cmpint(g_date_time_get_second(now), ==, 0);
}

static void first_minute_is_emitted(void) {
    test_time = g_date_time_new_utc(2026, 9, 15, 12, 34, 59);
    ClockService *service = g_object_new(CLOCK_SERVICE_TYPE, NULL);
    guint ticks = 0;
    g_signal_connect(service, "tick", G_CALLBACK(on_tick), &ticks);
    g_assert_true(tick_sync(service));
    g_assert_cmpuint(ticks, ==, 0);
    g_date_time_unref(test_time);
    test_time = g_date_time_new_utc(2026, 9, 15, 12, 35, 0);
    g_assert_false(tick_sync(service));
    g_assert_cmpuint(ticks, ==, 1);
    g_assert_cmpuint(scheduled_seconds, ==, 60);
    g_object_unref(service);
    g_date_time_unref(test_time);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/clock/first-minute", first_minute_is_emitted);
    return g_test_run();
}
