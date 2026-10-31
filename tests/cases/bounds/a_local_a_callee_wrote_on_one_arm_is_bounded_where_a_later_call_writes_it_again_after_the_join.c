void put(void ***p);
void tick(void);
void use(int ***p);
void run(int c) {
    int ***v = 0;
    void *pv = &v;
    if (c) {
        put(pv);
    }
    v = 0;
    tick();
    use(v);
}
