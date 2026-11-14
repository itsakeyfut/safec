int look(char *s);

int f(char *s, char *t) {
    if (s == 0) {
        return 0;
    }
    if (t == 0) {
        return 0;
    }
    return look(s) + look(t);
}
