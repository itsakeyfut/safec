void *malloc(int n);
void stash(char (*p)[8]);
void later(void);
void use1(int *p);
void run(void) {
    int *w = 0;
    int ***box = malloc(8);
    if (box == 0) {
        return;
    }
    *box = &w;
    int ***c = box;
    void *pc = &c;
    stash(pc);
    w = 0;
    later();
    use1(w);
}
