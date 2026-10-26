void *malloc(int n);
void use(int *****p);
void run(void ***m) {
    void *pv = m;
    int ***x = pv;
    int *****box = malloc(8);
    if (box == 0) { return; }
    *box = &x;
    void *pb = box;
    use(pb);
}
