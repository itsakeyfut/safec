void put(void ***p);
void use(int ***p);
void run(void) {
    int ***v = 0;
    void *pv = &v;
    put(pv);
    use(v);
}
