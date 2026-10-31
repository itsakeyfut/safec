void put4(int ****p);
void tick(void);
void use(int ***p);
void run(void ***p) {
    int ***v = 0;
    put4(&v);
    void *t = p;
    v = t;
    tick();
    use(v);
}
