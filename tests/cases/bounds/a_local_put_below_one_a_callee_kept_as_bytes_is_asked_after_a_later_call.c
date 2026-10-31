void keep(char *p);
void other(void);
void use1(int *p);
void run(void) {
    int x = 0;
    int *w = &x;
    int **v = 0;
    void *pv = &v;
    keep(pv);
    v = &w;
    other();
    use1(w);
}
