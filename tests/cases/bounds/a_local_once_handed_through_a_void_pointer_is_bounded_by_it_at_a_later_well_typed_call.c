void put(void ***p);
void put4(int ****p);
void use(int ***p);
void run(void) {
    int ***v = 0;
    void *pv = &v;
    put(pv);
    v = 0;
    put4(&v);
    use(v);
}
