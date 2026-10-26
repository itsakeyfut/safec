void use(int ***p);
void run(void **m) {
    void *pv = &m;
    use(pv);
}
