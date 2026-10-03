void log_line(void);
int main(int argc, char **argv) {
    if (argc < 1) {
        return 0;
    }
    if (argv == 0) {
        return 0;
    }
    log_line();
    char *name = argv[0];
    if (name == 0) {
        return 0;
    }
    return name[0];
}
