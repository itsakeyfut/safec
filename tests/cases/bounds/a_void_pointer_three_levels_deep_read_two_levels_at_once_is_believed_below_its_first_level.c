void run(void ***m) {
    void *pm = m;
    int ***pi = pm;
    if (pi != 0) {
        if (*pi != 0) {
            int *b = **pi;
            if (b != 0) {
                *b = 1;
            }
        }
    }
}
int main(void) {
    char c;
    void *v = &c;
    void **w = &v;
    run(&w);
    return 0;
}
