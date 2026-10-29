int main(void) {
    int x = 0;
    int *m[16];
    void *v = m;
    char *c = v;
    void *w = c + 8;
    int **p = w;
    *p = &x;
    int *written = m[1];
    int *unwritten = m[8];
    return written == unwritten;
}
