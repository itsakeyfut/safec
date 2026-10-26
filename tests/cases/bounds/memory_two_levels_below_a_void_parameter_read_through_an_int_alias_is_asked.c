void run(void ***m) {
    if (m != 0) {
        void **t = *m;
        if (t != 0) {
            void *pt = t;
            int **pi = pt;
            int *p = *pi;
            if (p != 0) {
                *p = 1;
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
