void clear(char *p);
void use(int **p);
void run(void) {
    int **v = 0;
    void *pv = &v;
    clear(pv);
    use(v);
}
