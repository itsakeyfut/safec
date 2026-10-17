void *malloc(int n);
void free(void *p);
void log_ptr(void **n);
void release(void **n);

int f(void) {
    void **head = 0;
    int i = 0;
    while (i < 3) {
        void **node = malloc(8);
        if (node == 0) return 0;
        *node = head;
        head = node;
        i = i + 1;
    }
    void **first = head;
    if (!first) return 0; void **n = *first;
    log_ptr(first);
    int c = 0;
    if (n) { void *z = *n; c = 1; }
    return c;
}
