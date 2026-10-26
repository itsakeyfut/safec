void use(int ****p);
void run(void ***m) {
    void *pv = m;
    int ***x = pv;
    use(&x);
}
