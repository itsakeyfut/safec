struct Node {
    struct Node *next;
    int value;
};

struct A {
    struct B *b;
};

struct B {
    struct A *a;
};

void keep(struct Node *node);

void push(struct Node *_Nonnull head, struct Node *node) {
    head->next = node;
}

void link(struct A *_Nonnull a, struct B *b) {
    a->b = b;
}

int handed(void) {
    struct Node head;
    struct Node other;
    keep(&head);
    head.next = &other;
    return 0;
}

int main(void) {
    return handed();
}
