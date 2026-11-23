void *malloc(unsigned long long size);
void free(void *pointer);

struct Node {
    int value;
    int *next;
};

int read_after_free(void) {
    struct Node *n = malloc(sizeof *n);
    if (!n) {
        return 1;
    }
    n->value = 5;
    free(n);
    return n->value;
}

int write_past_the_end(void) {
    struct Node *n = malloc(4);
    if (!n) {
        return 1;
    }
    n->next = 0;
    free(n);
    return 0;
}

int read_a_pointer_member_never_written(void) {
    struct Node *n = malloc(sizeof *n);
    if (!n) {
        return 1;
    }
    n->value = 1;
    int r = *n->next;
    free(n);
    return r;
}

int main(void) {
    return read_after_free() + write_past_the_end() + read_a_pointer_member_never_written();
}
