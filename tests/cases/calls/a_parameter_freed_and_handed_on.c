void free(void *p);
void run(int *p);

void finish(int *p) {
    free(p);
    run(p);
}
