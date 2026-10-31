void put(char (*p)[8]);
void use(int **p);
void run(void) {
    int **v = 0;
    void *pv = &v;
    put(pv);
    use(v);
}
