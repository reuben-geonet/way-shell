#include <adwaita.h>
#include <errno.h>
#include <unistd.h>

static const guint8 input[] = "a fragmented IPC message";
static guint8 written[sizeof(input)];
static gsize input_offset, written_offset;
static gboolean interrupted;
static ssize_t fragmented_read(int fd, void *buffer, size_t count) {
    if (!interrupted) { interrupted = TRUE; errno = EINTR; return -1; }
    gsize size = MIN(MIN(count, 2), sizeof(input) - input_offset);
    memcpy(buffer, input + input_offset, size);
    input_offset += size;
    return size;
}
static ssize_t fragmented_write(int fd, const void *buffer, size_t count) {
    if (!interrupted) { interrupted = TRUE; errno = EINTR; return -1; }
    gsize size = MIN(MIN(count, 2), sizeof(written) - written_offset);
    memcpy(written + written_offset, buffer, size);
    written_offset += size;
    return size;
}
#define read fragmented_read
#define write fragmented_write
#include "../src/services/window_manager_service/sway/sway_client.c"

static void partial_reads(void) {
    guint8 result[sizeof(input)] = {0};
    input_offset = 0;
    interrupted = FALSE;
    g_assert_cmpint(socket_read(0, result, sizeof(result)), ==, sizeof(result));
    g_assert_cmpmem(result, sizeof(result), input, sizeof(input));
    g_assert_cmpint(socket_read(0, result, sizeof(result)), <, 0);
}
static void partial_writes(void) {
    written_offset = 0;
    interrupted = FALSE;
    g_assert_cmpint(socket_write(0, (guint8 *)input, sizeof(input)), ==, sizeof(input));
    g_assert_cmpmem(written, sizeof(written), input, sizeof(input));
    g_assert_cmpint(socket_write(0, (guint8 *)input, sizeof(input)), <, 0);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/sway/partial-reads", partial_reads);
    g_test_add_func("/sway/partial-writes", partial_writes);
    return g_test_run();
}
