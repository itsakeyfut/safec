void *malloc(int n);
void free(void *p);
void log_line(void);

int build(void) {
    void **head;
    void **node;
    void **cur;
    void **next;
    head = malloc(8);
    if (head == 0) {
        return 0;
    }
    *head = 0;
    node = malloc(8);
    if (node == 0) {
        free(head);
        return 0;
    }
    *node = 0;
    cur = head;
    next = *cur;
    while (next != 0) {
        cur = next;
        next = *cur;
    }
    *cur = node;
    log_line();
    *node = 0;
    free(node);
    free(head);
    return 1;
}
