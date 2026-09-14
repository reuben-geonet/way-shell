#include "quick_settings_grid_power_profiles_menu.h"

#include <adwaita.h>

#include "../../../../services/power_profiles_service/power_profiles_service.h"
#include "../../quick_settings_menu_widget.h"
#include "../quick_settings_grid_button.h"

typedef struct _MenuOption {
    GtkBox *container;
    GtkButton *button;
    GtkImage *icon;
    GtkLabel *label;
    char *profile;
} MenuOption;

static void on_click(GtkButton *button, MenuOption *self) {
    PowerProfilesService *pps = power_profiles_service_get_global();
    power_profiles_service_set_profile(pps, self->profile);
}

static void free_menu_option(gpointer data) {
    MenuOption *option = data;
    g_free(option->profile);
    g_free(option);
}

static MenuOption *new_menu_option(char *profile) {
    MenuOption *self = g_malloc0(sizeof(MenuOption));

    self->profile = g_strdup(profile);

    self->container = GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
    gtk_widget_add_css_class(GTK_WIDGET(self->container),
                             "quick-settings-menu-option-power-profiles");

    self->button = GTK_BUTTON(gtk_button_new());
    gtk_widget_set_hexpand(GTK_WIDGET(self->button), TRUE);

    g_object_set_data_full(G_OBJECT(self->button), "profile-option", self,
                           free_menu_option);

    // connect button signal
    g_signal_connect(self->button, "clicked", G_CALLBACK(on_click), self);

    GtkBox *button_contents =
        GTK_BOX(gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));

    // create and append icon
    const char *icon_name = power_profiles_service_profile_to_icon(profile);
    self->icon = GTK_IMAGE(gtk_image_new_from_icon_name(icon_name));
    gtk_box_append(button_contents, GTK_WIDGET(self->icon));

    self->label = GTK_LABEL(gtk_label_new(profile));
    gtk_label_set_ellipsize(self->label, PANGO_ELLIPSIZE_END);
    gtk_widget_set_halign(GTK_WIDGET(self->label), GTK_ALIGN_START);
    gtk_box_append(button_contents, GTK_WIDGET(self->label));

    gtk_button_set_child(self->button, GTK_WIDGET(button_contents));
    gtk_box_append(self->container, GTK_WIDGET(self->button));

    return self;
}

enum signals { password_entry_revealed, signals_n };

typedef struct _QuickSettingsGridPowerProfilesMenu {
    GObject parent_instance;
    QuickSettingsMenuWidget menu;
    PowerProfilesService *service;
} QuickSettingsGridPowerProfilesMenu;
G_DEFINE_TYPE(QuickSettingsGridPowerProfilesMenu,
              quick_settings_grid_power_profiles_menu, G_TYPE_OBJECT);

static void on_profiles_changed(PowerProfilesService *pps, GArray *profiles,
                                QuickSettingsGridPowerProfilesMenu *self);

// stub out dispose, finalize, init and class init functions for GObject
static void quick_settings_grid_power_profiles_menu_dispose(GObject *object) {
    QuickSettingsGridPowerProfilesMenu *self =
        QUICK_SETTINGS_GRID_POWER_PROFILES_MENU(object);

    // disconnect from signals
    if (self->service)
        g_signal_handlers_disconnect_by_data(self->service, self);
    g_clear_object(&self->service);
    g_clear_object(&self->menu.container);
    G_OBJECT_CLASS(quick_settings_grid_power_profiles_menu_parent_class)->dispose(object);
}

static void quick_settings_grid_power_profiles_menu_finalize(GObject *object) {
    G_OBJECT_CLASS(quick_settings_grid_power_profiles_menu_parent_class)->finalize(object);
}

static void quick_settings_grid_power_profiles_menu_class_init(
    QuickSettingsGridPowerProfilesMenuClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = quick_settings_grid_power_profiles_menu_dispose;
    object_class->finalize = quick_settings_grid_power_profiles_menu_finalize;
}

static void quick_settings_grid_power_profiles_menu_init_layout(
    QuickSettingsGridPowerProfilesMenu *self) {
    GArray *profiles = power_profiles_service_get_profiles(self->service);

    // for each profile add it to self->menu.options
    for (int i = 0; i < profiles->len; i++) {
        char *profile = g_array_index(profiles, char *, i);
        MenuOption *option = new_menu_option(profile);
        gtk_box_append(self->menu.options, GTK_WIDGET(option->container));
    }
}

static void on_profiles_changed(PowerProfilesService *pps, GArray *profiles,
                                QuickSettingsGridPowerProfilesMenu *self) {
    // remove all options, this will unref the parent container as well.
    GtkWidget *child =
        gtk_widget_get_first_child(GTK_WIDGET(self->menu.options));
    while (child) {
        gtk_box_remove(self->menu.options, child);
        child = gtk_widget_get_first_child(GTK_WIDGET(self->menu.options));
    }
    // Update the existing visible menu, retaining its container.
    quick_settings_grid_power_profiles_menu_init_layout(self);
}

static void quick_settings_grid_power_profiles_menu_init(
    QuickSettingsGridPowerProfilesMenu *self) {
    self->service = g_object_ref(power_profiles_service_get_global());
    quick_settings_menu_widget_init(&self->menu, false);
    g_object_ref_sink(self->menu.container);
    gtk_image_set_from_icon_name(self->menu.icon, "power-profile-balanced-symbolic");
    gtk_label_set_text(self->menu.title, "Performance");
    quick_settings_grid_power_profiles_menu_init_layout(self);

    // setup signal to power profiles service profiles-changed
    g_signal_connect(self->service, "profiles-changed", G_CALLBACK(on_profiles_changed),
                     self);
}

GtkWidget *quick_settings_grid_power_profiles_menu_get_widget(
    QuickSettingsGridPowerProfilesMenu *self) {
    return GTK_WIDGET(self->menu.container);
}

void quick_settings_grid_power_profiles_menu_on_reveal(
    QuickSettingsGridButton *button, gboolean is_revealed) {
    g_debug(
        "quick_settings_grid_power_profiles_menu.c:qs_grid_power_profiles_menu_"
        "on_reveal() called");
}