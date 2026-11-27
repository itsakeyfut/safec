static char buffer[16];

int f(int x) {
    static int n = sizeof((int[]){x, 2});
    static char *offset = buffer + sizeof((int[]){1, 2});
    return n + *offset;
}
