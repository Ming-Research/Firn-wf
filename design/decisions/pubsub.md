# Publish and subscribe

How firn orders and delivers pub/sub messages: what subscribed connections, the registries and script publications share.

### decision pubsub-commits-through-one-event-hub
Membership changes and complete reply groups commit under one event hub, and one dispatcher that never touches the network delivers the committed frames in order to each connection's output inbox.
- rejected: an agent proposed that publishers send to the subscribers' sockets directly; the owner rejected it, because a slow socket would then delay the other subscribers.
- rejected: an agent proposed that publishers release the registry hold and then each append to the recipients' inboxes; the owner rejected it, because the order in which concurrent publishers wake up would then change the order of delivery.