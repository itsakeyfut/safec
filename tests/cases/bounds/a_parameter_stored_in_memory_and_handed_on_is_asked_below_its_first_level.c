void *malloc(int n);
void free(void *p);
void use(int ***p);
void run(void **m) {
    void ***box = malloc(8);
    if (box == 0) {
        return;
    }
    *box = m;
    void *pb = box;
    use(pb);
}
