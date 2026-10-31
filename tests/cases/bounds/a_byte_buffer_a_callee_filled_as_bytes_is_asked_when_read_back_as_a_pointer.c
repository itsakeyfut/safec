void put(char *p);
void use(int **p);
void run(void) {
    char buf[8];
    void *pb = buf;
    put(pb);
    int ***t = pb;
    int **v = *t;
    use(v);
}
