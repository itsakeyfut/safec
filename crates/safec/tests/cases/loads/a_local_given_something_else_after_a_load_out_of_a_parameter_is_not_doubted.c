void release_all(void);
void use(int *p);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    q = 0;
    release_all();
    use(q);
    return 0;
}
