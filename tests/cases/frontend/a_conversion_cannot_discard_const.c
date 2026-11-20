int main(void) {
    char c;
    const char *s;
    char *t;
    const char *u;
    s = &c;
    t = s;
    u = &c;
    u = t;
    return 0;
}
