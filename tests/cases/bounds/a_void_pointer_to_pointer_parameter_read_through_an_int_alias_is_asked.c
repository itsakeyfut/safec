void run(void **m) {
    if (m != 0) {
        void *pm = m;
        int **pi = pm;
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}

int main(void) {
    char c;
    void *v = &c;
    run(&v);
    return 0;
}
