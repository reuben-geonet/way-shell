#include <glib.h>
#include "../src/services/ipc_service/ipc_protocol.h"
/* Fixture replacements must not export production names: Rust may place a
 * needed bridge function in the same archive object as the real audio API. */
#define wire_plumber_service_get_global fixture_audio_get_global
#define wire_plumber_service_get_default_sink fixture_audio_get_default_sink
#define wire_plumber_service_set_volume fixture_audio_set_volume
/* Keep the production dispatcher private while testing its volume handler.
 * Unused UI sections are discarded by the test linker's --gc-sections. */
#include "../src/services/ipc_service/ipc_service.c"

static WirePlumberService *fake_service;
static WirePlumberServiceNode *fake_sink;
static double requested_volume = -1;

WirePlumberService *wire_plumber_service_get_global(void) { return fake_service; }
WirePlumberServiceNode *wire_plumber_service_get_default_sink(WirePlumberService *self) {
    return fake_sink;
}
void wire_plumber_service_set_volume(WirePlumberService *self,
                                    const WirePlumberServiceNode *node, double volume) {
    g_assert_true(self == fake_service);
    g_assert_true(node == fake_sink);
    requested_volume = volume;
}

static void request_validation(void) {
    IPCVolumeSet request = {0};
    uint8_t bytes[9] = {0};
    for (size_t size = 0; size < 4; size++)
        g_assert_false(ipc_decode_request(bytes, size, &request));
    for (uint32_t opcode = 0; opcode <= 33; opcode++) {
        ipc_write_u32(bytes, opcode);
        size_t expected = opcode == 4 ? 8 : 4;
        g_assert_true(ipc_decode_request(bytes, expected, &request));
        g_assert_cmpuint(request.header.type, ==, opcode);
        g_assert_false(ipc_decode_request(bytes, expected + 1, &request));
        g_assert_false(ipc_decode_request(bytes, expected - 1, &request));
    }
    ipc_write_u32(bytes, UINT32_MAX);
    g_assert_false(ipc_decode_request(bytes, 4, &request));
    ipc_write_u32(bytes, 4);
    uint32_t invalid[] = {0x7fc00000, 0x7f800000, 0xff800000, 0xbf000000, 0x40000000};
    for (unsigned i = 0; i < G_N_ELEMENTS(invalid); i++) {
        ipc_write_u32(bytes + 4, invalid[i]);
        g_assert_false(ipc_decode_request(bytes, 8, &request));
    }
    ipc_write_u32(bytes + 4, 0x3f000000);
    g_assert_true(ipc_decode_request(bytes, 8, &request));
    g_assert_cmpfloat(request.volume, ==, 0.5f);
}

static void volume_acknowledgement(void) {
    IPCVolumeSet request = {.header.type = IPC_CMD_VOLUME_SET, .volume = 0.5f};
    g_test_expect_message(NULL, G_LOG_LEVEL_CRITICAL, "*failed to get wireplumber*");
    g_assert_false(ipc_cmd_volume_set(&request));
    g_test_assert_expected_messages();
    fake_service = (WirePlumberService *)&request;
    g_assert_false(ipc_cmd_volume_set(&request));
    WirePlumberServiceNode node = {0};
    fake_sink = &node;
    g_assert_true(ipc_cmd_volume_set(&request));
    g_assert_cmpfloat(requested_volume, ==, 0.5);
}

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/ipc/request-validation", request_validation);
    g_test_add_func("/ipc/volume-acknowledgement", volume_acknowledgement);
    return g_test_run();
}
