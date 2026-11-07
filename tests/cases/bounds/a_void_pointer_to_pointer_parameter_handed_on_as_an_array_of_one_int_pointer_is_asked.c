void use(int *(*pi)[1]) {
    if (pi != 0) {
        int *p = (*pi)[0];
        if (p != 0) {
            *p = 1;
        }
    }
}

void run(void **m) {
    void *pm = m;
    use(pm);
}

int main(void) {
    char c;
    void *v = &c;
    run(&v);
    return 0;
}
