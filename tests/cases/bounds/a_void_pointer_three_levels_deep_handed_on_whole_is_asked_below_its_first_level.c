void use(int ***p);
void run(void ***m) {
    void *pm = m;
    use(pm);
}
