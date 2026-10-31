void put(char (*p)[8]);
void use1(int *p);
void run(void) {
    int *w = 0;
    int **v = &w;
    void *pv = &v;
    put(pv);
    use1(w);
}
