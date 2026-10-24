void free(int *p);
void g(void);
void h(void);

int f(int *p, int c) {
    free(p);
    c ? g() : h();
    (g(), c);
    for (g(); c; g()) {
        c = 0;
    }
    return 0;
}
